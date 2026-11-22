//! Loading feedback shared by the timeline and the room list: ghost bubbles and rows crossed by a soft light, a small ring, one status line.
//! Nothing here repaints at rest: the callers draw it only while something loads.
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use matrix_desktop::{DesignTokens, i18n};
use std::time::{Duration, Instant};

/// Ghosts wait this long (a room that opens at once shows none), the status line a little longer; both fade in.
pub const GHOST_DELAY: Duration = Duration::from_millis(120);
pub const LINE_DELAY: Duration = Duration::from_millis(250);
pub const FADE: Duration = Duration::from_millis(150);
/// From here the line says that it is taking long.
pub const SLOW: Duration = Duration::from_secs(8);
/// One pass of the light, one turn of the ring (seconds).
const SWEEP: f64 = 1.4;
const TURN: f64 = 0.9;
/// Height of the status row and diameter of its ring.
pub const LINE_H: f32 = 30.;
const RING: f32 = 14.;

/// Where the history request of the room stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Fetch {
    Busy,
    #[default]
    Done,
    Failed,
}

/// What the message pane shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pane {
    Rows,
    Loading,
    Empty,
    Error,
}

/// `shown`: the rows are on screen. Rows held hidden while the opening position is looked for (first unread, saved place) still count as loading.
pub fn pane(fetch: Fetch, events: usize, shown: bool) -> Pane {
    match (events, fetch) {
        (1.., _) if shown => Pane::Rows,
        (1.., _) | (0, Fetch::Busy) => Pane::Loading,
        (0, Fetch::Failed) => Pane::Error,
        (0, Fetch::Done) => Pane::Empty,
    }
}

/// The room list shows ghost rows only before the first sync answered: an account without rooms stays empty.
pub fn list_waiting(rooms: usize, synced: bool) -> bool {
    rooms == 0 && !synced
}

/// Since when something has been loading; forgotten as soon as it is not.
#[derive(Default)]
pub struct Clock(Option<Instant>);

impl Clock {
    pub fn elapsed(&mut self, active: bool, now: Instant) -> Option<Duration> {
        if !active {
            self.0 = None;
            return None;
        }
        Some(now.duration_since(*self.0.get_or_insert(now)))
    }
}

/// Opacity of the ghosts and of the status line, and whether the line is the «long» one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub ghosts: f32,
    pub line: f32,
    pub slow: bool,
}

pub fn stage(elapsed: Duration) -> Stage {
    let fade = |delay: Duration| ((elapsed.as_secs_f32() - delay.as_secs_f32()) / FADE.as_secs_f32()).clamp(0., 1.);
    Stage { ghosts: fade(GHOST_DELAY), line: fade(LINE_DELAY), slow: elapsed >= SLOW }
}

/// When a loading view must be drawn again: the next frame while it moves or fades, else only when the line changes.
pub fn redraw_in(elapsed: Duration, reduce_motion: bool) -> Option<Duration> {
    if !reduce_motion || elapsed < LINE_DELAY + FADE {
        Some(Duration::from_millis(8))
    } else {
        SLOW.checked_sub(elapsed).filter(|d| !d.is_zero())
    }
}

/// Animation clock of a frame: `None` with Reduce Motion (still ghosts, ring that does not turn).
pub fn motion(ui: &egui::Ui) -> Option<f64> {
    (!crate::mobile::reduce_motion()).then(|| ui.input(|i| i.time))
}

/// Where the light is on its way across the pane, 0..1, slow at both ends.
pub fn sweep(time: f64) -> f32 {
    let p = (time / SWEEP).rem_euclid(1.) as f32;
    p * p * (3. - 2. * p)
}

/// Strength 0..1 of the light at `pos`: a soft band, slanted, that enters `area` from one side and leaves from the other.
pub fn glow(area: Rect, pos: Pos2, sweep: f32) -> f32 {
    const SLANT: f32 = 0.35;
    let band = (area.width() * 0.45).max(1.);
    let span = area.width() + area.height() * SLANT;
    let center = -band + sweep * (span + 2. * band);
    let u = pos.x - area.left() + (pos.y - area.top()) * SLANT;
    let k = 1. - ((u - center).abs() / band).min(1.);
    k * k * (3. - 2. * k)
}

/// Start angle and length of the ring's arc; without motion a still three-quarter ring.
pub fn ring_arc(time: Option<f64>) -> (f32, f32) {
    use std::f32::consts::TAU;
    match time {
        Some(t) => ((t / TURN).rem_euclid(1.) as f32 * TAU, TAU * 0.3),
        None => (-TAU / 4., TAU * 0.75),
    }
}

pub fn arc_points(center: Pos2, radius: f32, (start, len): (f32, f32)) -> Vec<Pos2> {
    let n = ((len / std::f32::consts::TAU * 32.).ceil() as usize).max(2);
    (0..=n).map(|i| center + radius * Vec2::angled(start + len * i as f32 / n as f32)).collect()
}

/// A ghost message: `own` ones sit on the sender's side, the last of an incoming group has the avatar beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ghost {
    pub rect: Rect,
    pub own: bool,
    pub avatar: Option<Rect>,
}

/// Ghost bubbles from the bottom of `area` upwards, as many as fit (at most nine), laid out like the real rows: `line` is the height of a text line.
pub fn bubbles(area: Rect, line: f32) -> Vec<Ghost> {
    // Bottom to top: own, share of the widest bubble, text lines.
    const SHAPES: [(bool, f32, f32); 9] = [(false, 0.56, 1.), (false, 0.84, 2.), (true, 0.44, 1.), (false, 0.68, 3.), (true, 0.78, 2.), (true, 0.36, 1.), (false, 0.48, 1.), (true, 0.62, 2.), (false, 0.74, 1.)];
    let (side, pad, avatar) = (12., 0.7 * line, 1.6 * line);
    let widest = ((area.width() - 2. * side - avatar - 8.) * 0.85).max(avatar);
    let mut out = Vec::new();
    let mut bottom = area.bottom() - 8.;
    for (i, (own, share, lines)) in SHAPES.into_iter().enumerate() {
        let size = Vec2::new((widest * share).max(avatar), lines * line + pad);
        if bottom - size.y < area.top() + LINE_H + 24. {
            break;
        }
        let left = if own { area.right() - side - size.x } else { area.left() + side + avatar + 8. };
        let rect = Rect::from_min_size(egui::pos2(left, bottom - size.y), size);
        let grouped = i > 0 && SHAPES[i - 1].0 == own;
        let avatar = (!own && !grouped).then(|| Rect::from_min_size(egui::pos2(area.left() + side, bottom - avatar), Vec2::splat(avatar)));
        out.push(Ghost { rect, own, avatar });
        bottom -= size.y + if SHAPES.get(i + 1).is_some_and(|s| s.0 == own) { 3. } else { 10. };
    }
    out
}

/// Ghost rows of a list, top to bottom: avatar, name bar, preview bar.
pub fn list_rows(area: Rect, row: f32) -> Vec<(Rect, Rect, Rect)> {
    const WIDTHS: [(f32, f32); 5] = [(0.46, 0.82), (0.62, 0.58), (0.38, 0.9), (0.54, 0.7), (0.7, 0.5)];
    let n = ((area.height() / row).floor() as usize).min(10);
    let avatar = row - 12.;
    let left = area.left() + 16. + avatar + 12.;
    let free = (area.right() - 16. - left).max(8.);
    (0..n).map(|i| {
        let top = area.top() + i as f32 * row;
        let (name, preview) = WIDTHS[i % WIDTHS.len()];
        let bar = |y: f32, h: f32, w: f32| Rect::from_min_size(egui::pos2(left, top + y * row), Vec2::new(free * w, h * row));
        (Rect::from_min_size(egui::pos2(area.left() + 16., top + 6.), Vec2::splat(avatar)), bar(0.2, 0.22, name * 0.7), bar(0.58, 0.18, preview))
    }).collect()
}

/// A rounded shape as vertical slices, each corner coloured by `ink`: the gradient crosses the shape.
pub fn slices(rect: Rect, radius: f32, ink: impl Fn(Pos2) -> Color32) -> egui::Mesh {
    let r = radius.min(rect.width() / 2.).min(rect.height() / 2.);
    let n = ((rect.width() / 4.).ceil() as u32).max(2);
    let mut mesh = egui::Mesh::default();
    for i in 0..=n {
        let x = rect.left() + rect.width() * i as f32 / n as f32;
        // How far the round corner pulls the edge in at this x.
        let dx = (r - (x - rect.left()).min(rect.right() - x)).max(0.);
        let inset = r - (r * r - dx * dx).sqrt();
        for y in [rect.top() + inset, rect.bottom() - inset] {
            let pos = egui::pos2(x, y);
            mesh.colored_vertex(pos, ink(pos));
        }
        if i > 0 {
            let k = 2 * i;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    mesh
}

fn rgb(c: u32) -> Color32 {
    crate::list::rgb(c)
}

/// A stroke is never thinner than one physical pixel.
fn stroke(ui: &egui::Ui, width: f32, color: Color32) -> egui::Stroke {
    egui::Stroke::new(width.max(1. / ui.ctx().pixels_per_point()), color)
}

/// Fill of a ghost and colour of the light on it: a step off the background `bg` it lies on, so it reads in every theme.
/// The light brightens a dark ghost and thins a light one out towards the background (never past it: the ghost would vanish on the way).
fn inks(ui: &egui::Ui, bg: Color32, t: &DesignTokens, alpha: f32) -> (Color32, Color32) {
    let base = bg.lerp_to_gamma(rgb(t.secondary), 0.09);
    let light = if ui.visuals().dark_mode { rgb(t.secondary).gamma_multiply(0.1) } else { bg.gamma_multiply(0.6) };
    (base.gamma_multiply(alpha), light.gamma_multiply(alpha))
}

fn ghost(p: &egui::Painter, rect: Rect, radius: f32, area: Rect, (base, light): (Color32, Color32), sweep: Option<f32>) {
    p.rect_filled(rect, radius, base);
    let Some(s) = sweep else { return };
    let corners = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()];
    if corners.iter().any(|c| glow(area, *c, s) > 0.) {
        p.add(egui::Shape::mesh(slices(rect, radius, |pos| light.gamma_multiply(glow(area, pos, s)))));
    }
}

/// The ghost chat: bubbles on both sides, fading towards the top.
pub fn skeleton_timeline(ui: &egui::Ui, area: Rect, bg: Color32, line: f32, alpha: f32, time: Option<f64>, t: &DesignTokens) {
    if alpha <= 0. {
        return;
    }
    let p = ui.painter().with_clip_rect(area);
    let ghosts = bubbles(area, line);
    let sweep = time.map(sweep);
    for (i, g) in ghosts.iter().enumerate() {
        let inks = inks(ui, bg, t, alpha * (1. - 0.6 * i as f32 / ghosts.len() as f32));
        ghost(&p, crate::dir::flip(area, g.rect), 0.7 * line, area, inks, sweep);
        if let Some(a) = g.avatar {
            ghost(&p, crate::dir::flip(area, a), a.width() / 2., area, inks, sweep);
        }
    }
}

/// The ghost room list, fading towards the bottom. Named for screen readers by `label`.
pub fn skeleton_list(ui: &mut egui::Ui, bg: Color32, row: f32, alpha: f32, time: Option<f64>, label: &str, t: &DesignTokens) {
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, egui::Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, label));
    if alpha <= 0. {
        return;
    }
    let p = ui.painter().with_clip_rect(area);
    let rows = list_rows(area, row);
    let sweep = time.map(sweep);
    for (i, (avatar, name, preview)) in rows.iter().enumerate() {
        let inks = inks(ui, bg, t, alpha * (1. - 0.8 * i as f32 / rows.len() as f32));
        ghost(&p, crate::dir::flip(area, *avatar), avatar.width() / 2., area, inks, sweep);
        for bar in [name, preview] {
            ghost(&p, crate::dir::flip(area, *bar), bar.height() / 2., area, inks, sweep);
        }
    }
}

/// The indeterminate ring: a faint track and an arc that turns.
pub fn ring(ui: &egui::Ui, center: Pos2, radius: f32, alpha: f32, time: Option<f64>, t: &DesignTokens) {
    let width = radius / 3.5;
    ui.painter().circle_stroke(center, radius, stroke(ui, width, rgb(t.secondary).gamma_multiply(0.25 * alpha)));
    ui.painter().add(egui::Shape::line(arc_points(center, radius, ring_arc(time)), stroke(ui, width, rgb(t.accent).gamma_multiply(alpha))));
}

/// What the status line says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    Busy { slow: bool },
    Failed,
    Empty,
}

impl Line {
    pub fn text(self) -> &'static str {
        match self {
            Line::Busy { slow: false } => i18n::t("Loading messages…"),
            Line::Busy { slow: true } => i18n::t("Taking longer than usual…"),
            Line::Failed => i18n::t("Couldn't load messages"),
            Line::Empty => i18n::t("No messages yet"),
        }
    }
}

/// One status line centred in `rect`: the ring and its text, a failure and its retry button (true when pressed), or the quiet «empty» label.
/// `capsule` puts it on a pill, for a line that floats over the ghosts.
pub fn status_line(ui: &mut egui::Ui, rect: Rect, line: Line, capsule: bool, alpha: f32, time: Option<f64>, t: &DesignTokens) -> bool {
    if alpha <= 0. {
        return false;
    }
    let text = line.text();
    let lead = match line { Line::Busy { .. } => RING, Line::Failed => crate::metrics::button(), Line::Empty => crate::metrics::get().icon };
    let (pad, gap) = (12., 8.);
    let color = rgb(if line == Line::Failed { t.error } else { t.secondary }).gamma_multiply(alpha);
    let galley = crate::list::one_line(ui, text, 13., color, (rect.width() - 2. * pad - lead - gap - 16.).max(20.));
    let size = Vec2::new(pad + lead + gap + galley.size().x + pad, rect.height().min(LINE_H.max(lead)));
    let pill = Rect::from_center_size(rect.center(), size);
    if capsule {
        ui.painter().rect(pill, size.y / 2., rgb(t.surface).gamma_multiply(alpha), stroke(ui, 1., rgb(t.border).gamma_multiply(alpha)), egui::StrokeKind::Inside);
    }
    // Worked out for LTR: the mark first, then the text.
    let slot = crate::dir::flip(pill, Rect::from_min_size(egui::pos2(pill.left() + pad, pill.center().y - lead / 2.), Vec2::splat(lead)));
    ui.painter().galley(crate::dir::place(pill, egui::pos2(pill.left() + pad + lead + gap, pill.center().y - galley.size().y / 2.), galley.size().x), galley, color);
    let id = ui.scope_id().with(("loading-line", capsule));
    match line {
        Line::Busy { .. } => {
            ring(ui, slot.center(), RING / 2., alpha, time, t);
            ui.interact(pill, id, egui::Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, text));
            false
        }
        Line::Failed => {
            ui.interact(pill, id, egui::Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
            crate::icons::button_at(ui, slot, id.with("retry"), crate::icons::Icon::ArrowClockwise, false, crate::metrics::get().icon, i18n::t("Retry"), Some(rgb(t.accent)), true, alpha).clicked()
        }
        Line::Empty => {
            crate::icons::paint(ui.painter(), slot.center(), crate::icons::Icon::ChatCircleDots, false, lead, color);
            ui.interact(pill, id, egui::Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
            false
        }
    }
}

/// The message pane when it has no rows to show: ghosts and the line near the top while loading, else the empty label or the failure in the middle.
/// Asks for the next frame only while something moves. True when «Retry» was pressed.
pub fn pane_overlay(ui: &mut egui::Ui, area: Rect, bg: Color32, what: Pane, elapsed: Duration, line: f32, time: Option<f64>, t: &DesignTokens) -> bool {
    let middle = Rect::from_center_size(area.center(), Vec2::new(area.width(), LINE_H.max(crate::metrics::button())));
    match what {
        Pane::Rows => false,
        Pane::Loading => {
            let s = stage(elapsed);
            skeleton_timeline(ui, area, bg, line, s.ghosts, time, t);
            status_line(ui, Rect::from_min_size(area.left_top() + Vec2::new(0., 12.), Vec2::new(area.width(), LINE_H)), Line::Busy { slow: s.slow }, true, s.line, time, t);
            if let Some(d) = redraw_in(elapsed, time.is_none()) {
                ui.ctx().request_repaint_after(d);
            }
            false
        }
        Pane::Empty => status_line(ui, middle, Line::Empty, false, 1., time, t),
        Pane::Error => status_line(ui, middle, Line::Failed, true, 1., time, t),
    }
}

/// Debug builds: GNOBBI_DEBUG_ROOM_LOAD_DELAY_MS=<ms> holds every history page back that long, to see the loading states on a fast server.
/// `sent` marks a request going out; true while its answer must wait.
pub fn debug_hold(sent: bool) -> bool {
    #[cfg(debug_assertions)]
    {
        static DELAY: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
        static SENT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        let Some(delay) = *DELAY.get_or_init(|| matrix_desktop::hooks::var("GNOBBI_DEBUG_ROOM_LOAD_DELAY_MS").ok().and_then(|v| v.parse().ok()).map(Duration::from_millis)) else { return false };
        let mut at = SENT.lock().unwrap();
        if sent {
            *at = Some(Instant::now());
        }
        at.is_some_and(|t| t.elapsed() < delay)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = sent;
        false
    }
}

/// Test hook, debug builds only: GNOBBI_EGUI_VIEW=loading-<room|slow|page|failed-page|empty|error> draws that state alone, a ghost list beside it, with no account;
/// GNOBBI_DEBUG_LOADING_T=<seconds> is the frozen animation time.
#[cfg(debug_assertions)]
pub fn demo(ctx: &egui::Context, view: &str, pane_fill: Color32, list_fill: Color32, t: &DesignTokens) {
    let at: f64 = matrix_desktop::hooks::var("GNOBBI_DEBUG_LOADING_T").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5);
    let time = (!crate::mobile::reduce_motion()).then_some(at);
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("loading-demo")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        ui.painter().rect_filled(screen, 0., pane_fill);
        let mut area = screen;
        if screen.width() >= crate::SINGLE_WIDTH {
            let list = Rect::from_min_size(screen.min, Vec2::new(320., screen.height()));
            ui.painter().rect_filled(list, 0., list_fill);
            ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink2(Vec2::new(0., 12.))), |ui| skeleton_list(ui, list_fill, 52., 1., time, i18n::t("Loading conversations…"), t));
            area.min.x = list.right();
        } else if view == "loading-list" {
            ui.painter().rect_filled(screen, 0., list_fill);
            ui.scope_builder(egui::UiBuilder::new().max_rect(screen.shrink2(Vec2::new(0., 12.))), |ui| skeleton_list(ui, list_fill, 52., 1., time, i18n::t("Loading conversations…"), t));
            return;
        }
        let top = Rect::from_min_size(area.min, Vec2::new(area.width(), LINE_H + 2.));
        match view {
            "loading-page" => { status_line(ui, top, Line::Busy { slow: false }, false, 1., time, t); }
            "loading-failed-page" => { status_line(ui, top, Line::Failed, false, 1., time, t); }
            "loading-empty" => { pane_overlay(ui, area, pane_fill, Pane::Empty, Duration::ZERO, 20., time, t); }
            "loading-error" => { pane_overlay(ui, area, pane_fill, Pane::Error, Duration::ZERO, 20., time, t); }
            "loading-slow" => { pane_overlay(ui, area, pane_fill, Pane::Loading, SLOW, 20., time, t); }
            _ => { pane_overlay(ui, area, pane_fill, Pane::Loading, Duration::from_secs_f64(at), 20., time, t); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_slow_room_goes_from_nothing_to_ghosts_to_the_line_to_the_long_line_and_ends_in_rows_empty_or_error() {
        let t0 = Instant::now();
        let mut clock = Clock::default();
        let mut at = |fetch, events, shown, after: u64| {
            let what = pane(fetch, events, shown);
            (what, clock.elapsed(what == Pane::Loading, t0 + ms(after)).map(stage))
        };
        // Opened: nothing for the first instants, so a room that is already there never flickers.
        assert_eq!(at(Fetch::Busy, 0, false, 0), (Pane::Loading, Some(Stage { ghosts: 0., line: 0., slow: false })));
        assert_eq!(at(Fetch::Busy, 0, false, 100).1.unwrap().ghosts, 0.);
        let s = at(Fetch::Busy, 0, false, 200).1.unwrap();
        assert!(s.ghosts > 0. && s.ghosts < 1. && s.line == 0., "ghosts fade in first: {s:?}");
        let s = at(Fetch::Busy, 0, false, 300).1.unwrap();
        assert!(s.ghosts == 1. && s.line > 0. && s.line < 1., "then the line: {s:?}");
        assert_eq!(at(Fetch::Busy, 0, false, 450).1, Some(Stage { ghosts: 1., line: 1., slow: false }));
        // Pages arrive but the first unread is still looked for: the hidden rows keep the same loading view, the clock goes on.
        assert_eq!(at(Fetch::Busy, 40, false, 7_999).1, Some(Stage { ghosts: 1., line: 1., slow: false }));
        assert_eq!(at(Fetch::Done, 40, false, 8_000).1, Some(Stage { ghosts: 1., line: 1., slow: true }));
        // Loaded: rows, and the clock is forgotten.
        assert_eq!(at(Fetch::Done, 40, true, 9_000), (Pane::Rows, None));
        assert_eq!(at(Fetch::Busy, 0, false, 20_000).1, Some(stage(Duration::ZERO)), "the next room starts from zero");
        assert_eq!(at(Fetch::Done, 0, false, 20_500), (Pane::Empty, None));
        assert_eq!(at(Fetch::Failed, 0, false, 20_500), (Pane::Error, None));
    }

    #[test]
    fn rows_on_screen_are_never_covered_by_ghosts() {
        for fetch in [Fetch::Busy, Fetch::Done, Fetch::Failed] {
            for events in [1, 2, 500] {
                assert_eq!(pane(fetch, events, true), Pane::Rows, "{fetch:?} {events}");
            }
            assert_ne!(pane(fetch, 0, true), Pane::Rows);
        }
        assert_ne!(pane(Fetch::Done, 0, false), pane(Fetch::Busy, 0, false), "empty is not loading");
    }

    #[test]
    fn the_list_has_ghost_rows_only_before_the_first_sync() {
        assert!(list_waiting(0, false));
        assert!(!list_waiting(0, true), "an account without rooms");
        assert!(!list_waiting(3, false), "rooms from the cache");
    }

    #[test]
    fn a_still_view_is_redrawn_only_for_the_fades_and_the_long_line() {
        assert_eq!(redraw_in(ms(5_000), false), Some(ms(8)), "moving: every frame");
        assert_eq!(redraw_in(ms(100), true), Some(ms(8)), "fading in");
        assert_eq!(redraw_in(ms(5_000), true), Some(ms(3_000)), "still: when the line changes");
        assert_eq!(redraw_in(SLOW, true), None);
        assert_eq!(redraw_in(ms(60_000), true), None);
    }

    #[test]
    fn the_light_crosses_the_pane_once_per_period_and_starts_and_ends_outside() {
        let area = Rect::from_min_size(egui::pos2(100., 50.), Vec2::new(400., 600.));
        let mid = area.center();
        assert_eq!(sweep(0.), 0.);
        assert!((sweep(SWEEP + 0.2) - sweep(0.2)).abs() < 1e-4, "periodic");
        assert!((sweep(SWEEP / 2.) - 0.5).abs() < 1e-5);
        assert!(sweep(0.1) < 0.1 / SWEEP as f32, "eased: slow at the start");
        for corner in [area.left_top(), area.right_bottom(), mid] {
            assert_eq!((glow(area, corner, 0.), glow(area, corner, 1.)), (0., 0.), "no light left on the ghosts at the turn");
        }
        let seen: Vec<f32> = (0..=20).map(|i| glow(area, mid, i as f32 / 20.)).collect();
        assert!(seen.iter().all(|g| (0. ..=1.).contains(g)));
        assert!(seen.iter().cloned().fold(0., f32::max) > 0.95, "the band passes over the middle: {seen:?}");
        assert_ne!(glow(area, mid, sweep(0.5)), glow(area, mid, sweep(0.7)), "two moments, two lights");
        // Slanted: lower points are lit later.
        assert!(glow(area, area.left_top(), 0.2) > glow(area, area.left_bottom(), 0.2));
    }

    #[test]
    fn the_ring_turns_with_time_and_stands_still_without_motion() {
        let c = egui::pos2(50., 50.);
        let (a, b) = (arc_points(c, 7., ring_arc(Some(0.1))), arc_points(c, 7., ring_arc(Some(0.3))));
        assert_ne!(a[0], b[0], "the arc moved");
        assert_eq!(ring_arc(Some(0.1)).1, ring_arc(Some(0.3)).1, "same length");
        assert_eq!(ring_arc(Some(0.)).0, ring_arc(Some(TURN)).0, "one turn per period");
        assert_eq!(ring_arc(None), ring_arc(None));
        assert!(ring_arc(None).1 > ring_arc(Some(0.)).1, "the still ring is longer: it reads as a ring, not as a dash");
        for p in a.iter().chain(&b) {
            assert!(((*p - c).length() - 7.).abs() < 1e-3);
        }
    }

    #[test]
    fn ghost_bubbles_fit_every_width_alternate_sides_and_vary() {
        for (w, h) in [(300., 500.), (300. / 1.34, 420.), (402., 700.), (1100., 900.)] {
            let area = Rect::from_min_size(egui::pos2(20., 10.), Vec2::new(w, h));
            let g = bubbles(area, 20.);
            assert!((6..=9).contains(&g.len()), "{w}x{h}: {}", g.len());
            for b in &g {
                assert!(area.contains_rect(b.rect) && b.avatar.is_none_or(|a| area.contains_rect(a) && !a.intersects(b.rect)), "{w}: {b:?}");
                assert!(b.rect.top() >= area.top() + LINE_H + 12., "room for the status line");
                assert!(!b.own || b.avatar.is_none(), "avatars only beside incoming bubbles");
                // Sides: own bubbles hug the far edge, the others leave the avatar column.
                assert_eq!(b.own, b.rect.right() > area.right() - 13., "{w}: {b:?}");
            }
            assert!(g.iter().any(|b| b.own) && g.iter().any(|b| b.avatar.is_some()));
            let distinct = |f: fn(&Ghost) -> f32| { let mut v: Vec<i32> = g.iter().map(|b| f(b) as i32).collect(); v.sort(); v.dedup(); v.len() };
            assert!(distinct(|b| b.rect.width()) >= 5 && distinct(|b| b.rect.height()) >= 3);
            for pair in g.windows(2) {
                assert!(pair[1].rect.bottom() < pair[0].rect.top(), "no overlap");
            }
        }
        assert!(bubbles(Rect::from_min_size(Pos2::ZERO, Vec2::new(300., 60.)), 20.).is_empty(), "no room, no ghosts");
        // A bigger text line gives bigger ghosts.
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(400., 800.));
        assert!(bubbles(area, 26.)[0].rect.height() > bubbles(area, 20.)[0].rect.height());
    }

    #[test]
    fn ghost_list_rows_stay_inside_a_narrow_column() {
        for w in [220., 320., 402.] {
            let area = Rect::from_min_size(egui::pos2(0., 100.), Vec2::new(w, 640.));
            let rows = list_rows(area, 52.);
            assert_eq!(rows.len(), 10);
            for (avatar, name, preview) in &rows {
                assert!(area.contains_rect(*avatar) && area.contains_rect(*name) && area.contains_rect(*preview), "{w}");
                assert!(name.left() > avatar.right() && preview.top() > name.bottom());
            }
        }
    }

    #[test]
    fn the_lit_mesh_stays_inside_its_round_shape_and_changes_with_the_light() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(400., 600.));
        let rect = Rect::from_min_size(egui::pos2(60., 300.), Vec2::new(220., 54.));
        let lit = |s: f32| slices(rect, 14., |p| Color32::WHITE.gamma_multiply(glow(area, p, s)));
        let (a, b) = (lit(0.4), lit(0.6));
        assert!(a.is_valid());
        assert_eq!(a.vertices.len(), b.vertices.len());
        assert!(a.vertices.iter().zip(&b.vertices).any(|(x, y)| x.color != y.color), "the light moved");
        assert!(a.vertices.iter().all(|v| rect.expand(0.01).contains(v.pos)));
        // The first slice starts where the corner does: at mid height, not at the square corner.
        assert_eq!((a.vertices[0].pos.y, a.vertices[1].pos.y), (rect.top() + 14., rect.bottom() - 14.));
        // A circle (avatar): top and bottom meet at the sides.
        let dot = slices(Rect::from_min_size(Pos2::ZERO, Vec2::splat(32.)), 16., |_| Color32::WHITE);
        assert_eq!(dot.vertices[0].pos.y, dot.vertices[1].pos.y);
    }
}
