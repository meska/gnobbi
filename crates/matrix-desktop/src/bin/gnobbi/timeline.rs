//! Central column: the selected room's messages, read-only. Rows have different heights, so the
//! heights seen so far are cached and off-screen rows are replaced by empty space.
use crate::list::rgb;
use eframe::egui::{self, Align, Color32, CornerRadius, RichText, Vec2};
use matrix_desktop::{DemoEvent, DesignTokens, i18n};

/// Poll clicks of this frame: (poll start, option index).
#[derive(Default)]
pub struct Acts {
    pub vote: Option<(matrix_domain::EventId, usize)>,
    /// The play/pause button of a voice note was pressed.
    pub play: Option<matrix_domain::EventId>,
    /// Set by the app before drawing: the note being played (or loaded) and how far it is.
    pub now_playing: Option<(matrix_domain::EventId, bool, f32)>,
    pub open_link: Option<matrix_desktop::SafeWebUrl>,
    pub picture: Option<(matrix_domain::EventId, bool, String)>,
    /// The name or picture of a sender (or a mention) was clicked: the member to show.
    pub profile: Option<String>,
    /// The quote of a reply was clicked: the original to scroll to.
    pub reply_jump: Option<matrix_domain::EventId>,
    /// The message just jumped to, lit for a moment.
    pub flash: Option<matrix_domain::EventId>,
    /// The long press of this frame, if it landed on this list (set by `show`).
    pub press: Option<egui::Pos2>,
    /// Set by the app before drawing: the message the touch menu lifts over the dimmed chat, and how far (0..1).
    pub lift: Option<(matrix_domain::ScopedTimelineId, f32)>,
    /// Where that message's bubble is this frame.
    pub lift_rect: Option<egui::Rect>,
    /// The bubble a long press opened the menu on (the menu `show` returns names the message).
    pub held: Option<egui::Rect>,
    /// The viewport of the list.
    pub view: Option<egui::Rect>,
    /// Taps of this session on reaction pills not yet shown by the server (set by the app before drawing).
    pub wishes: Vec<matrix_desktop::Wish>,
    /// What the pills asked for this frame.
    pub react: crate::reactions::Acts,
}

/// Where the menu was asked for on the bubble `rect` this frame: a right click, or a long press (`acts.held` then carries the bubble).
/// Read from the input, not a widget over the bubble: a click-sensing widget on top would swallow the clicks of the link and the download button.
fn menu_at(ui: &egui::Ui, rect: egui::Rect, e: &DemoEvent, acts: &mut Acts) -> Option<egui::Pos2> {
    if acts.lift.as_ref().is_some_and(|(id, _)| *id == e.id) {
        acts.lift_rect = Some(rect);
    }
    let inside = |p: &egui::Pos2| rect.contains(*p) && ui.is_rect_visible(rect);
    let click = ui.input(|i| if i.pointer.button_clicked(egui::PointerButton::Secondary) { i.pointer.interact_pos() } else { None }).filter(inside);
    let press = acts.press.filter(inside);
    if press.is_some() {
        acts.held = Some(rect);
    }
    click.or(press)
}

/// Test hook (debug builds): GNOBBI_DEBUG_LONGPRESS_MSG=<index> long-presses that row once, a few seconds after the room is laid out;
/// a negative index counts from the newest message (-1).
#[cfg(debug_assertions)]
fn debug_press(i: usize, len: usize) -> bool {
    use std::sync::{OnceLock, atomic::{AtomicBool, Ordering}};
    static WANT: OnceLock<Option<i64>> = OnceLock::new();
    static SINCE: OnceLock<std::time::Instant> = OnceLock::new();
    static DONE: AtomicBool = AtomicBool::new(false);
    let Some(want) = *WANT.get_or_init(|| matrix_desktop::hooks::var("GNOBBI_DEBUG_LONGPRESS_MSG").ok()?.parse().ok()) else { return false };
    (if want < 0 { len as i64 + want } else { want }) == i as i64 && SINCE.get_or_init(std::time::Instant::now).elapsed().as_secs() >= 4 && !DONE.swap(true, Ordering::Relaxed)
}

pub(crate) fn link_parts(text: &str) -> Vec<(&str, Option<matrix_desktop::SafeWebUrl>)> {
    let mut parts = Vec::new();
    let mut start = 0;
    for link in matrix_desktop::message_links(text) {
        if start < link.range.start {
            parts.push((&text[start..link.range.start], None));
        }
        parts.push((&text[link.range.clone()], Some(link.url)));
        start = link.range.end;
    }
    if start < text.len() {
        parts.push((&text[start..], None));
    }
    parts
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    #[test]
    fn a_click_on_a_sender_opens_that_member_with_the_hand() {
        let (ctx, at, mut acts) = (egui::Context::default(), egui::pos2(10., 10.), super::Acts::default());
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut cursor = egui::CursorIcon::Default;
        for events in [vec![egui::Event::PointerMoved(at)], vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 100.))), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                let (_, resp) = ui.allocate_exact_size(egui::vec2(50., 50.), egui::Sense::click());
                super::sender_link(resp, "@bob:example.org", &mut acts);
            });
            out.textures_delta.clear();
            cursor = out.platform_output.cursor_icon;
        }
        assert_eq!(acts.profile.as_deref(), Some("@bob:example.org"));
        assert_eq!(cursor, egui::CursorIcon::PointingHand);
    }

    #[test]
    fn a_change_of_reactions_moves_the_cached_height_by_the_difference() {
        let (mut stored, id) = (std::collections::HashMap::new(), matrix_domain::EventId::new("$a"));
        // Pills appear under a row measured without them, grow to a second line, stay, then all go.
        assert_eq!(super::pill_shift(&mut stored, &id, 28., 100.), 128.);
        assert_eq!(super::pill_shift(&mut stored, &id, 28., 128.), 128., "unchanged: the measured height stands");
        assert_eq!(super::pill_shift(&mut stored, &id, 56., 128.), 156.);
        assert_eq!(super::pill_shift(&mut stored, &id, 0., 156.), 100.);
        assert!(stored.is_empty());
    }

    #[test]
    fn prepended_page_shifts_measured_heights() {
        let (mut rows, mut first) = (vec![10., 20.], None);
        assert_eq!(super::shift_for_prepend(&mut rows, &mut first, &[&3, &4], 0), 0);
        assert_eq!((rows.clone(), first), (vec![10., 20.], Some(3)));
        let mut first = Some(3);
        assert_eq!(super::shift_for_prepend(&mut rows, &mut first, &[&1, &2, &3, &4], 0), 2);
        assert_eq!((rows, first), (vec![56., 56., 10., 20.], Some(1)));
        // A thread: the root (30) stays first, the older replies land between it and the ones already measured.
        let (mut rows, mut first) = (vec![30., 10., 20.], Some(3));
        assert_eq!(super::shift_for_prepend(&mut rows, &mut first, &[&1, &2, &3, &4], 1), 2);
        assert_eq!((rows, first), (vec![30., 56., 56., 10., 20.], Some(1)));
    }

    #[test]
    fn ambiguous_sender_label_splits_into_name_and_id() {
        assert_eq!(super::split_id("Marco (@meska:example.org)", "@meska:example.org"), ("Marco", true));
        assert_eq!(super::split_id("Marco (ufficio)", "@meska:example.org"), ("Marco (ufficio)", false));
        assert_eq!(super::split_id("@meska:example.org", "@meska:example.org"), ("@meska:example.org", false));
        assert_eq!(super::split_id("Marco ()", ""), ("Marco ()", false));
    }

    #[test]
    fn links_preserve_surrounding_text_and_reject_unsafe_urls() {
        let parts = super::link_parts("Ciao https://example.org/a e javascript:alert(1)");
        assert_eq!(parts.iter().map(|(s, _)| *s).collect::<String>(), "Ciao https://example.org/a e javascript:alert(1)");
        assert_eq!(parts.iter().filter(|(_, url)| url.is_some()).count(), 1);
        assert_eq!(parts.iter().find_map(|(_, url)| url.as_ref()).unwrap().as_str(), "https://example.org/a");
    }
}

pub type Pictures = std::collections::HashMap<matrix_domain::EventId, egui::TextureHandle>;
pub use crate::avatar::Avatars;

/// Makes `resp` (a sender's name, picture or mention pill) open that member's card: hand cursor, screen-reader label, click.
fn sender_link(resp: egui::Response, user: &str, acts: &mut Acts) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, i18n::t("Open profile")));
    let resp = crate::metrics::clickable(resp);
    if resp.clicked() { acts.profile = Some(user.to_owned()); }
    resp
}

/// Round avatar of `size` (see `crate::avatar::paint`).
fn avatar(ui: &mut egui::Ui, size: f32, pic: &crate::avatar::Avatar, initials: &str, font: f32, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_avatar(ui, rect, pic, initials, font, t);
    resp
}

fn paint_avatar(ui: &egui::Ui, rect: egui::Rect, pic: &crate::avatar::Avatar, initials: &str, font: f32, t: &DesignTokens) {
    crate::avatar::paint(ui, rect.center(), rect.width(), rgb(matrix_desktop::AVATAR_FALLBACK), pic, initials, font, rgb(t.avatar_text));
}

/// Gap between groups / inside one, bubble padding, width.
const GAP: f32 = 8.;
/// Band above the oldest row, always there (no jump when it fills): the «loading older messages» line is drawn in it.
const TOP: f32 = crate::loading::LINE_H + 2.;
const TIGHT: f32 = 2.;
const PAD_X: f32 = 12.;
const PAD_Y: f32 = 7.;
/// Bubble corners: full, and where two bubbles of a group touch.
const ROUND: u8 = 14;
const JOINED: u8 = 4;
/// Time (and read circles) at the bottom right of the bubble: text size, row height, space kept from the text on the same line.
const STAMP: f32 = 11.;
const FOOT_H: f32 = 15.;
const FOOT_GAP: f32 = 10.;
/// Messages of one sender closer than this form a group.
const GROUP_GAP_MS: u64 = 10 * 60_000;

/// What grouping needs from a row: sender (`None` = own message), time, whether it is drawn as a bubble.
type GroupKey<'a> = (Option<&'a str>, u64, bool);

fn group_key(e: &DemoEvent) -> GroupKey<'_> {
    ((!e.outgoing).then(|| e.sender.as_ref().map_or("", |s| s.id.as_str())), e.timestamp_ms, !e.status && !e.redacted)
}

/// Whether a bubble is glued to the one above: same known sender, close in time, same day; status and deleted rows (no bubble) break a group.
fn joined(above: GroupKey, below: GroupKey, new_day: impl FnOnce() -> bool) -> bool {
    above.2 && below.2 && above.0 == below.0 && below.0 != Some("") && below.1.saturating_sub(above.1) <= GROUP_GAP_MS && !new_day()
}
const MAX_BUBBLE: f32 = 720.;
const BODY: f32 = 14.;
const GUESS: f32 = 56.;

/// Measured row heights for the open room.
#[derive(Default)]
pub struct Heights {
    rows: Vec<f32>,
    /// Id of the first row `rows` was measured for: a page prepended on top shifts every index.
    first: Option<matrix_domain::ScopedTimelineId>,
    /// The newest message is in (or near) view: what lets a read receipt go out.
    pub at_bottom: bool,
    /// A freshly opened room stays unpainted until its rows are measured and the bottom anchor holds: no guessed-height frames on screen.
    ready: bool,
    passes: u8,
    /// Where the reader was (first visible row + px into it; None = at bottom): set every frame once the room is laid out.
    pos: Option<(matrix_domain::EventId, f32)>,
    /// Position to open the room at; stays pending (pane hidden, older pages loading) until its row is loaded; the session drops it when the row cannot come (then the room opens at the bottom).
    pub restore: Option<(matrix_domain::EventId, f32)>,
    /// History results applied and start time while `restore` waits for its row: bound of the wait.
    pub restore_wait: (u8, Option<std::time::Instant>),
    /// The round button was pressed: scroll (smoothly) until the bottom is reached.
    go_bottom: bool,
    /// `restore` is the first unread row: it opens a quarter of the viewport below the top, with some context above.
    pub restore_unread: bool,
    /// The row the «new messages» line stands above (kept until the room is left).
    pub divider: Option<matrix_domain::EventId>,
    /// Height of the reaction pills each row was last laid out with: a row out of view is not measured again, so a change moves its cached height by the difference.
    pills: std::collections::HashMap<matrix_domain::EventId, f32>,
    /// Rows whose bottom edge is in view this frame, as (index, event id): what the read marker may follow.
    pub visible: Vec<(usize, matrix_domain::EventId)>,
    /// Unread rows are in view and their receipt is pending: keep drawing until it goes out.
    pub watching: bool,
    /// Where the history request stands (set by the caller before drawing), and since when the pane / the older page has been loading.
    pub fetch: crate::loading::Fetch,
    opening: crate::loading::Clock,
    paging: crate::loading::Clock,
    /// «Retry» was pressed on a failed load.
    pub retry: bool,
}

impl Heights {
    /// The reader's position, only when the room is laid out (a half-opened room must not overwrite the saved one).
    pub fn position(&self) -> Option<Option<(matrix_domain::EventId, f32)>> {
        self.ready.then(|| self.pos.clone())
    }
}

/// Reading position per room, kept in `scroll_pos.json` of the account's profile folder: the first visible event id and px offset into it, or at-bottom. Only event ids are stored.
#[derive(Default)]
pub struct ScrollMemory(std::collections::BTreeMap<String, Option<(String, f32)>>);

impl ScrollMemory {
    pub fn from_json(text: &str) -> Self {
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str(text) else { return Self::default() };
        let pos = |v: &serde_json::Value| {
            if v["at_bottom"].as_bool().unwrap_or(true) { return Some(None); }
            Some(Some((v["event"].as_str()?.to_owned(), v["offset"].as_f64()? as f32)))
        };
        Self(map.iter().filter_map(|(room, v)| Some((room.clone(), pos(v)?))).collect())
    }

    pub fn to_json(&self) -> String {
        let map: serde_json::Map<_, _> = self.0.iter().map(|(room, p)| (room.clone(), match p {
            Some((event, offset)) => serde_json::json!({ "event": event, "offset": offset, "at_bottom": false }),
            None => serde_json::json!({ "at_bottom": true }),
        })).collect();
        serde_json::Value::Object(map).to_string()
    }

    pub fn get(&self, room: &str) -> Option<(matrix_domain::EventId, f32)> {
        self.0.get(room)?.as_ref().map(|(e, o)| (matrix_domain::EventId::new(e.as_str()), *o))
    }

    /// True when the stored position changed (so the file needs writing).
    pub fn set(&mut self, room: &str, pos: Option<(matrix_domain::EventId, f32)>) -> bool {
        let new = pos.map(|(e, o)| (e.as_str().to_owned(), o.round()));
        let changed = self.0.get(room) != Some(&new);
        if changed { self.0.insert(room.to_owned(), new); }
        changed
    }
}

/// Opening pass is over: nothing moved and we sit at the bottom, or too many passes (a long room never reaches bottom within tolerance).
fn open_done(changed: bool, at_bottom: bool, passes: u8) -> bool {
    (!changed && at_bottom) || passes >= 8
}

/// True when the message was right-clicked (or long-pressed).
const READER_SIZE: f32 = 15.;
const READER_GAP: f32 = 2.;
const READER_MAX: usize = 5;

/// Width of the circles (plus the «+N» label when over the maximum) so the bubble shrink-wraps around them.
fn readers_width(n: usize) -> f32 {
    let shown = n.min(READER_MAX);
    if n == 0 { 0. } else { 6. + shown as f32 * (READER_SIZE + READER_GAP) + if n > READER_MAX { 16. } else { 0. } }
}

/// Small round avatars of the people whose newest read message is this one; the name shows on hover.
fn read_circles(ui: &mut egui::Ui, readers: &[matrix_engine::history::HistorySender], avatars: &Avatars, t: &DesignTokens) {
    if readers.is_empty() { return; }
    ui.add_space(4.);
    for r in readers.iter().take(READER_MAX) {
        let name = r.name.as_deref().unwrap_or(&r.id);
        let initials: String = name.trim_start_matches('@').chars().take(1).collect::<String>().to_uppercase();
        let pic = crate::avatar::lookup(avatars, r.avatar.as_deref());
        avatar(ui, READER_SIZE, &pic, &initials, 9., t).on_hover_text(name);
    }
    if readers.len() > READER_MAX {
        ui.label(RichText::new(format!("+{}", readers.len() - READER_MAX)).size(10.).color(rgb(t.secondary)));
    }
}

fn row(ui: &mut egui::Ui, e: &DemoEvent, pictures: &Pictures, avatars: &Avatars, acts: &mut Acts, label: Option<&str>, (above, below): (bool, bool), group: bool, day: Option<&str>, readers: &[matrix_engine::history::HistorySender], t: &DesignTokens) -> (Option<egui::Pos2>, bool, bool) {
    if let Some(day) = day {
        ui.add_space(8.);
        ui.vertical_centered(|ui| ui.label(RichText::new(day).size(11.).color(rgb(t.secondary))));
        ui.add_space(8.);
    }
    if e.status {
        // Quiet centred line: small round avatar + grey text, no bubble.
        let font = egui::FontId::proportional(11.);
        let avail = ui.available_width();
        // A line longer than the column is cut with an ellipsis: unwrapped, it widened the whole timeline past the visible edge.
        let status_text = e.status_row.as_ref().map_or_else(|| e.text.clone(), matrix_desktop::row_text::status_row_text);
        let text_w = ui.painter().layout_no_wrap(status_text.clone(), font, Color32::WHITE).size().x.min((avail - 36.).max(40.));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.;
            let pic = crate::avatar::lookup(avatars, e.sender.as_ref().and_then(|s| s.avatar.as_deref()));
            let has_avatar = e.sender.is_some();
            ui.add_space(((avail - text_w - if has_avatar { 18. } else { 0. }) / 2.).max(0.));
            if let Some(s) = &e.sender {
                let initials: String = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
                avatar(ui, 12., &pic, &initials, 7., t);
            }
            ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(&status_text)).size(11.).color(rgb(t.secondary))).truncate());
        });
        return (None, false, false);
    }
    if e.redacted {
        // Compact deleted row like the GPUI client: «Name: label … time», no bubble; right-click still opens the menu.
        let gutter = group && !e.outgoing && e.sender.is_some();
        let layout = if e.outgoing { crate::dir::row_rev(Align::Center) } else { crate::dir::row(Align::Center) };
        let r = ui.with_layout(layout, |ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            ui.add_space(if gutter { 52. } else { 12. });
            let small = |s: &str| RichText::new(s.to_owned()).size(12.).color(rgb(t.secondary));
            if let Some(name) = label {
                ui.label(RichText::new(matrix_desktop::row_text::redacted_name(name)).size(12.).strong().color(rgb(t.accent)));
            }
            ui.label(small(i18n::t("Message deleted")));
            ui.label(small(&e.time));
        });
        return (menu_at(ui, r.response.rect, e, acts), false, false);
    }
    // The side margin (12) and, in group rooms, the avatar column (32 + 8) are not available to the bubble: without them a wide bubble overflowed a narrow column and every row shifted right.
    let max = bubble_max(ui.available_width(), group && !e.outgoing);
    let layout = if e.outgoing { crate::dir::row_rev(Align::TOP) } else { crate::dir::row(Align::TOP) };
    let mut opened = None;
    // Media with reactions: the pills straddle the media's bottom edge inside the bubble (Telegram), not a line under it.
    let shown = reactions_of(e, &acts.wishes).into_owned();
    let over = pills_over_media(e);
    let mut bubble = egui::Rect::NOTHING;
    let mut save = false;
    let mut thread = false;
    ui.with_layout(layout, |ui| {
        ui.add_space(12.);
        // Group rooms: the sender's round avatar sits beside the last bubble of a group, at its bottom; the others keep its column.
        let column = (group && !e.outgoing).then(|| {
            let rect = ui.allocate_exact_size(Vec2::splat(32.), egui::Sense::hover()).0;
            ui.add_space(8.);
            rect
        });
        let r = |glued: bool| if glued { JOINED } else { ROUND };
        // Only the corners on the sender's side shrink where the bubbles touch.
        let radius = if e.outgoing != crate::dir::rtl() { CornerRadius { nw: ROUND, ne: r(above), sw: ROUND, se: r(below) } } else { CornerRadius { nw: r(above), ne: ROUND, sw: r(below), se: ROUND } };
        let flashed = matches!((&e.id.id, &acts.flash), (matrix_domain::TimelineId::Server(a), Some(b)) if a == b);
        // 1-3 emoji alone: Telegram-style big glyph with no bubble (a reply quote keeps the bubble around it).
        let big = (e.media.is_none() && e.poll.is_none() && e.thread.is_none() && e.attachment_name.is_none() && !e.utd && e.styles.is_empty() && e.mentions.is_empty())
            .then(|| matrix_desktop::big_emoji::scale(&e.text)).flatten();
        // A sticker alone is drawn like on Telegram: no bubble, smaller than a photo. "Sticker" is the engine's label of `m.sticker`.
        let sticker = e.text.is_empty() && e.media.as_ref().is_some_and(|m| m.label == "Sticker");
        let bare = (big.is_some() || sticker) && e.reply_to.is_none() && !e.highlighted && !flashed;
        let fill = if bare { Color32::TRANSPARENT } else if e.highlighted || flashed { rgb(t.mention_row) } else if e.outgoing { rgb(t.outgoing) } else { rgb(t.surface) };
        let margin = if bare { egui::Margin::symmetric(0, 2) } else { egui::Margin::symmetric(PAD_X as i8, PAD_Y as i8) };
        // The copy the touch menu lifts over the dimmed chat (see `show`) casts a soft shadow.
        let lifted = acts.lift.as_ref().filter(|(id, _)| !bare && *id == e.id && ui.layer_id() == crate::touch_menu::lift_layer()).map_or(0., |(_, by)| *by);
        let shadow = egui::Shadow { offset: [0, 4], blur: 20, spread: 0, color: Color32::from_black_alpha((64. * lifted) as u8) };
        let frame = egui::Frame::NONE.fill(fill).corner_radius(radius).inner_margin(margin).shadow(shadow).show(ui, |ui| {
            // Shrink-wrap: as wide as the longest of text, sender and time (at most `max`), not the whole column.
            let wide = e.media.is_some() || e.poll.is_some() || e.thread.is_some() || e.attachment_name.is_some();
            let inner = max - 2. * PAD_X;
            let measure = |text: &str, size: f32| ui.painter().layout(text.to_owned(), egui::FontId::proportional(size), Color32::WHITE, inner).size().x;
            let (time, mark) = e.stamp_parts();
            let foot_w = measure(time, STAMP) + mark.map_or(0., |_| STAMP + 2. + READER_GAP) + readers_width(readers.len());
            let body = (!e.text.is_empty() && e.poll.is_none()).then(|| {
                let size = BODY * big.unwrap_or(1.);
                let mut job = egui::text::LayoutJob::simple(if big.is_some() { e.text.trim().to_owned() } else { e.text.clone() }, egui::FontId::proportional(size), ui.visuals().text_color(), inner);
                job.sections[0].format.line_height = Some(if big.is_some() { size * 1.25 } else { 20. });
                ui.painter().layout_job(job)
            });
            let (text_w, tail_w) = body.as_ref().map_or((0., 0.), |g| (g.size().x, g.rows.last().map_or(0., |r| r.rect().width())));
            // The time shares the last text line when it fits beside it (the bubble does not grow taller); else it gets its own line.
            let mut inline = body.is_some() && e.thread.is_none() && big.is_none() && tail_w + FOOT_GAP + foot_w <= inner;
            let mut w = text_w.max(if inline { tail_w + FOOT_GAP + foot_w } else { foot_w });
            if let Some(name) = label {
                w = w.max(measure(name, 12.));
            }
            let quote = e.reply_to.as_ref().map(|r| {
                use matrix_engine::history::ReplyState as S;
                let (Some(author), S::Resolved) = (r.author.as_deref(), r.state) else {
                    return (i18n::t(match r.state { S::Loading => "Loading…", S::Deleted => "Message deleted", S::Encrypted => "Encrypted message", _ => "Message unavailable" }).to_owned(), None);
                };
                (crate::list::clean_single_line(author), r.preview.as_deref().map(|p| crate::list::clean_single_line(if r.placeholder { i18n::t(p) } else { p })))
            });
            if let Some((a, p)) = &quote {
                w = w.max(measure(a, 12.)).max(p.as_deref().map_or(0., |p| measure(p, 12.)).min(220.));
            }
            ui.set_width(if wide { inner.min(320.).max(w) } else { w.min(inner) + 1. });
            // The bubble sits on its side by the interface's direction; what is inside follows the message's own.
            let inner_rtl = crate::dir::content_rtl(&e.text);
            ui.with_layout(crate::dir::col_of(inner_rtl), |ui| {
                if let Some(name) = label {
                    // The display name alone; the user id is the tooltip, and stands beside the name (small, grey) only when the engine added it to tell two members apart.
                    let id = e.sender.as_ref().map_or("", |s| s.id.as_str());
                    let (name, twin) = split_id(name, id);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.;
                        sender_link(ui.add(egui::Label::new(RichText::new(name).size(12.).strong().color(rgb(t.accent))).sense(egui::Sense::click())), id, acts).on_hover_text(id);
                        if twin {
                            ui.add(egui::Label::new(RichText::new(id).size(10.).color(rgb(t.secondary))).truncate());
                        }
                    });
                }
                if let (Some((author, preview)), Some(r)) = (&quote, &e.reply_to) {
                    // Telegram-style quote: accent bar, original author, one line of its text; a click scrolls to the original.
                    let q = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.;
                        let (bar, _) = ui.allocate_exact_size(Vec2::new(3., 32.), egui::Sense::hover());
                        ui.painter().rect_filled(bar, 1., rgb(t.accent));
                        ui.with_layout(crate::dir::col_of(inner_rtl), |ui| {
                            ui.spacing_mut().item_spacing.y = 0.;
                            ui.add(egui::Label::new(RichText::new(author).size(12.).strong().color(rgb(t.accent))).truncate());
                            if let Some(p) = preview {
                                ui.add(egui::Label::new(RichText::new(p).size(12.).color(rgb(t.secondary))).truncate());
                            }
                        });
                    });
                    let hit = crate::metrics::clickable(ui.interact(q.response.rect, ui.id().with(("quote", &e.id.id)), egui::Sense::click()));
                    if hit.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if hit.clicked() { acts.reply_jump = Some(r.event_id.clone()); }
                    ui.add_space(4.);
                }
                if let (Some(media), matrix_domain::TimelineId::Server(id)) = (&e.media, &e.id.id) {
                    match pictures.get(id) {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let side = if sticker { 192. } else { 320. };
                            let scale = (max.min(side) / size.x).min(1.).min(side / size.y);
                            if crate::metrics::clickable(ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale).corner_radius(CornerRadius::same(8)).sense(egui::Sense::click()))).clicked() && matches!(media.kind, matrix_engine::history::MediaKind::Image | matrix_engine::history::MediaKind::Video) {
                                acts.picture = Some((id.clone(), media.kind == matrix_engine::history::MediaKind::Video, e.attachment_name.clone().unwrap_or_else(|| i18n::t("image").into())));
                            }
                        }
                        None => {
                            if crate::metrics::clickable(ui.add(egui::Label::new(RichText::new(media.translated(|k| i18n::t(k).to_owned())).color(rgb(t.secondary))).sense(egui::Sense::click()))).clicked() && media.kind == matrix_engine::history::MediaKind::Video { acts.picture = Some((id.clone(), true, e.attachment_name.clone().unwrap_or_else(|| i18n::t("video").into()))); }
                        }
                    }
                    if over && !shown.is_empty() {
                        crate::reactions::show(ui, e, &shown, group, true, egui::Rect::NOTHING, acts.press, &mut acts.react, t);
                    }
                }
                if let (Some(_), matrix_domain::TimelineId::Server(id)) = (&e.audio, &e.id.id) {
                    let (playing, progress) = acts.now_playing.as_ref().filter(|(p, ..)| p == id).map_or((false, 0.), |(_, pl, pr)| (*pl, *pr));
                    ui.horizontal(|ui| {
                        if crate::icons::button(ui, if playing { crate::icons::Icon::Pause } else { crate::icons::Icon::Play }, true, if playing { i18n::t("Pause") } else { i18n::t("Play") }).clicked() {
                            acts.play = Some(id.clone());
                        }
                        ui.add(egui::ProgressBar::new(progress).desired_width(140.).desired_height(8.));
                    });
                }
                if let Some(name) = &e.attachment_name
                    && !e.redacted
                    && ui.button(crate::icons::with_label(crate::icons::Icon::DownloadSimple, name)).clicked()
                {
                    save = true;
                }
                if let (Some(poll), matrix_domain::TimelineId::Server(id)) = (&e.poll, &e.id.id)
                    && !e.redacted
                {
                    ui.label(RichText::new(&poll.question).strong());
                    ui.add_space(4.);
                    for (ix, answer) in poll.answers.iter().enumerate() {
                        let share = if poll.results_visible() { poll.share(ix) } else { 0. };
                        let r = ui.add_enabled(!poll.ended, egui::Button::new(matrix_desktop::poll_text::option_label(poll, ix)).wrap().selected(answer.own).min_size(Vec2::new(inner.min(200.), crate::metrics::get().button - 6.)));
                        let bar = crate::dir::flip(r.rect, egui::Rect::from_min_size(r.rect.left_bottom() - Vec2::new(0., 3.), Vec2::new(r.rect.width() * share, 3.)));
                        ui.painter().rect_filled(bar, 1., rgb(t.accent));
                        if r.clicked() {
                            acts.vote = Some((id.clone(), ix));
                        }
                    }
                    ui.label(RichText::new(matrix_desktop::poll_text::footer(poll)).size(11.).color(rgb(t.secondary)));
                } else if e.redacted {
                    ui.label(RichText::new(i18n::t("Message deleted")).italics().color(rgb(t.secondary)));
                } else if !e.text.is_empty() {
                    let mut pills: Vec<_> = matrix_desktop::mention_gaps::fitting(&e.text, &e.mentions)
                        .filter_map(|m| match &m.kind { matrix_engine::rich_text::MentionKind::User { sender, .. } => Some((m.range.clone(), sender)), _ => None })
                        .collect();
                    pills.sort_by_key(|(r, _)| r.start);
                    let mut at = 0;
                    pills.retain(|(r, _)| { let ok = r.start >= at; if ok { at = r.end; } ok });
                    let parts = link_parts(&e.text);
                    if !pills.is_empty() || parts.iter().any(|(_, url)| url.is_some()) {
                        // Segments between pills keep their links; a pill is avatar + accent name, as in the GPUI client.
                        let mut segs: Vec<(&str, Option<&matrix_engine::history::HistorySender>)> = Vec::new();
                        let mut from = 0;
                        for (r, s) in &pills { segs.push((&e.text[from..r.start], None)); segs.push((&e.text[r.clone()], Some(*s))); from = r.end; }
                        segs.push((&e.text[from..], None));
                        let tail = crate::dir::wrapped(ui, inner_rtl, |ui| {
                            ui.spacing_mut().item_spacing.x = 0.;
                            for (seg, who) in segs {
                                if let Some(s) = who {
                                    let pic = crate::avatar::lookup(avatars, s.avatar.as_deref());
                                    let initials: String = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
                                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.), egui::Sense::click());
                                    paint_avatar(ui, rect, &pic, &initials, 9., t);
                                    sender_link(resp, &s.id, acts);
                                    sender_link(ui.add(egui::Label::new(RichText::new(seg).size(BODY).strong().color(rgb(t.accent))).sense(egui::Sense::click())), &s.id, acts);
                                    continue;
                                }
                                for (text, url) in link_parts(seg) {
                                    if let Some(url) = url {
                                        if ui.link(RichText::new(text).size(BODY).color(rgb(t.accent)).underline()).clicked() {
                                            acts.open_link = Some(url);
                                        }
                                    } else {
                                        ui.label(RichText::new(text).size(BODY).line_height(Some(20.)));
                                    }
                                }
                            }
                            if inner_rtl { ui.max_rect().right() - ui.cursor().right() } else { ui.cursor().left() - ui.max_rect().left() }
                        }).inner;
                        // Pills and links are laid out here, not in `body`: decide from the real last line.
                        inline = e.thread.is_none() && tail + FOOT_GAP + foot_w <= ui.available_width();
                    } else if let Some(body) = body {
                        ui.label(body);
                    }
                }
                if let Some(th) = &e.thread {
                    ui.horizontal(|ui| {
                        if let Some(s) = &th.latest_sender {
                            let pic = crate::avatar::lookup(avatars, s.avatar.as_deref());
                            let initials: String = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
                            avatar(ui, 20., &pic, &initials, 9., t);
                        }
                        if ui.link(i18n::plural(th.num_replies as u64, "{n} reply", "{n} replies")).clicked() {
                            thread = true;
                        }
                    });
                }
                ui.add_space(if inline { -FOOT_H } else if bare { 0. } else { 2. });
                // Bottom right of the bubble (the bubble is shrink-wrapped to its widest line, so this is its right edge).
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), FOOT_H), crate::dir::row_of(!inner_rtl, Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.x = READER_GAP;
                    // On the bare chat background the time sits on a small pill of the bubble colour, so contrast is the same in every theme.
                    let pill = if bare { egui::Frame::NONE.fill(rgb(t.surface)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(6, 0)) } else { egui::Frame::NONE };
                    pill.show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = READER_GAP;
                        ui.with_layout(crate::dir::row_of(!inner_rtl, Align::Center), |ui| {
                            // Right to left: the delivery mark is the rightmost, the time left of it.
                            if let Some(mark) = mark {
                                let ink = if mark == matrix_desktop::StampMark::Failed { rgb(t.error) } else { rgb(t.secondary) };
                                ui.label(crate::icons::rich(crate::icons::stamp_icon(mark), false, STAMP + 2.).color(ink));
                            }
                            ui.label(RichText::new(time).size(STAMP).color(rgb(t.secondary)));
                            // Right to left: the circles sit left of the time.
                            read_circles(ui, readers, avatars, t);
                        });
                    });
                });
            });
        });
        if let (Some(column), false, Some(s)) = (column, below, &e.sender) {
            let initials: String = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
            let pic = crate::avatar::lookup(avatars, s.avatar.as_deref());
            let rect = egui::Rect::from_min_size(egui::pos2(column.left(), frame.response.rect.bottom() - 32.), Vec2::splat(32.));
            paint_avatar(ui, rect, &pic, &initials, 12., t);
            sender_link(ui.interact(rect, ui.scope_id().with(("sender", &e.id.id)), egui::Sense::click()), &s.id, acts);
        }
        opened = menu_at(ui, frame.response.rect, e, acts);
        bubble = frame.response.rect;
    });
    if !over {
        crate::reactions::show(ui, e, &shown, group, false, bubble, acts.press, &mut acts.react, t);
    }
    (opened, save, thread)
}

/// A picture, sticker or video (not a file or audio): its reactions straddle the media's bottom edge.
fn pills_over_media(e: &DemoEvent) -> bool {
    use matrix_engine::history::MediaKind::{Image, Video};
    matches!(e.id.id, matrix_domain::TimelineId::Server(_)) && e.media.as_ref().is_some_and(|m| matches!(m.kind, Image | Video))
}

/// The widest a bubble gets in a column `avail` wide (`avatar`: a group's incoming one leaves room for the avatar).
fn bubble_max(avail: f32, avatar: bool) -> f32 {
    ((avail - 12. - if avatar { 40. } else { 0. }) * 0.85).clamp(120., MAX_BUBBLE)
}

/// The cached height of a row whose pills now take `ph`: moved by the difference from what they took when it was last laid out
/// (rows out of view are not measured again). `stored` remembers the pill height per row, only while there are pills.
fn pill_shift(stored: &mut std::collections::HashMap<matrix_domain::EventId, f32>, id: &matrix_domain::EventId, ph: f32, row: f32) -> f32 {
    let old = stored.get(id).copied().unwrap_or(0.);
    if (ph - old).abs() <= 0.5 {
        return row;
    }
    if ph > 0. { stored.insert(id.clone(), ph); } else { stored.remove(id); }
    (row + ph - old).max(0.)
}

/// The reactions to draw under `e`: the server's with the taps of this session on their way laid over.
fn reactions_of<'a>(e: &'a DemoEvent, wishes: &[matrix_desktop::Wish]) -> std::borrow::Cow<'a, [matrix_engine::history::Reaction]> {
    #[cfg(debug_assertions)]
    if !e.status && !e.redacted && let Some(fake) = crate::reactions::fixture() {
        return fake.into();
    }
    match &e.id.id {
        matrix_domain::TimelineId::Server(id) if !wishes.is_empty() => matrix_desktop::overlay_reactions(wishes, id, &e.reactions).into(),
        _ => (&e.reactions[..]).into(),
    }
}

/// «Name (@id)» as the engine writes an ambiguous sender: the name alone and whether the id was there.
fn split_id<'a>(label: &'a str, id: &str) -> (&'a str, bool) {
    match label.strip_suffix(')').and_then(|l| l.strip_suffix(id)).and_then(|l| l.strip_suffix(" (")) {
        Some(name) if !id.is_empty() => (name, true),
        _ => (label, false),
    }
}

/// «New messages»: accent label and a thin accent line to the right edge, above the first unread row.
fn new_messages_line(ui: &mut egui::Ui, t: &DesignTokens) {
    let accent = rgb(t.accent);
    let label = i18n::t("New messages");
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.), egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    let text = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.), accent);
    let mid = rect.center().y;
    let (from, to) = (rect.left() + 12., rect.right() - 12.);
    let line = |a: f32, b: f32| ui.painter().line_segment([egui::pos2(a, mid), egui::pos2(b, mid)], egui::Stroke::new(1., accent));
    if crate::dir::rtl() {
        ui.painter().galley(egui::pos2(to - text.size().x, mid - text.size().y / 2.), text.clone(), accent);
        line(from, to - text.size().x - 8.);
    } else {
        ui.painter().galley(egui::pos2(from, mid - text.size().y / 2.), text.clone(), accent);
        line(from + text.size().x + 8., to);
    }
}

/// Older page on top: the measured heights move with their rows, else every row takes a neighbour's height and the scroll jumps.
/// `ids` are the rows after the `head` pinned ones (a thread's root stays first while older replies arrive below it).
fn shift_for_prepend<T: PartialEq + Clone>(rows: &mut Vec<f32>, first: &mut Option<T>, ids: &[&T], head: usize) -> usize {
    let at = first.as_ref().and_then(|f| ids.iter().position(|id| *id == f)).unwrap_or(0);
    let head = head.min(rows.len());
    rows.splice(head..head, std::iter::repeat_n(GUESS, at));
    *first = ids.first().map(|id| (*id).clone());
    at
}

/// Draws the rows in view; returns the index of the first one in view, for paging back.
/// `head` is `None` for a room; in a thread it is the number of rows pinned on top (its root) and the rows gather at the bottom, beside the composer.
pub fn show(ui: &mut egui::Ui, unread: u64, events: &[DemoEvent], history: &matrix_desktop::DemoHistory, heights: &mut Heights, pictures: &Pictures, avatars: &Avatars, acts: &mut Acts, members: u64, readers: &std::collections::HashMap<usize, Vec<matrix_engine::history::HistorySender>>, t: &DesignTokens, jump: Option<usize>, head: Option<usize>) -> (usize, Option<(matrix_domain::ScopedTimelineId, egui::Pos2)>, Option<(matrix_domain::EventId, String)>, Option<matrix_domain::EventId>) {
    // Touch: the 44 pt minimum is for buttons and fields, not for every row of the history.
    if crate::mobile::touch() {
        ui.spacing_mut().interact_size.y = egui::style::Spacing::default().interact_size.y;
    }
    // One long press per frame, and only when it lands on this list (not on a panel, a menu or a viewer above it).
    acts.press = crate::touch_menu::long_press(ui.ctx()).filter(|p| ui.clip_rect().contains(*p) && ui.ctx().layer_id_at(*p) == Some(ui.layer_id()));
    let mut menu = None;
    let mut save = None;
    let mut thread = None;
    // Heights are kept across width changes (rows in view are re-measured): clearing them to GUESS collapsed the content height and the scroll offset jumped on every resize step.
    let pinned = head.unwrap_or(0).min(events.len());
    let prepended = shift_for_prepend(&mut heights.rows, &mut heights.first, &events[pinned..].iter().map(|e| &e.id).collect::<Vec<_>>(), pinned);
    heights.rows.resize(events.len(), GUESS);
    let mut first = 0;
    let hide = !heights.ready && jump.is_none() && !events.is_empty();
    let mut changed = false;
    let mut area = egui::ScrollArea::vertical().auto_shrink(false);
    let mut restoring = false;
    let mut anchor_missing = false;
    let glued = |i: usize| i > 0 && i < events.len() && joined(group_key(&events[i - 1]), group_key(&events[i]), || matrix_desktop::time_fmt::starts_day(Some(events[i - 1].timestamp_ms), events[i].timestamp_ms));
    let top_of = |rows: &[f32], at: usize| TOP + (0..at).map(|i| rows[i] + if glued(i + 1) { TIGHT } else { GAP }).sum::<f32>();
    let position = |id: &matrix_domain::EventId| events.iter().position(|e| e.id.id == matrix_domain::TimelineId::Server(id.clone()));
    if !hide && prepended > 0 {
        // An older page arrived: the rows on top push everything down and egui would paint this frame at the old offset (the wrong rows for a frame, 100 ms when idle). Keep the same row at the same offset, or stay at the bottom.
        if let Some((id, offset)) = heights.pos.clone() && let Some(at) = position(&id) {
            area = area.vertical_scroll_offset(top_of(&heights.rows, at) + offset);
        } else if heights.at_bottom {
            area = area.vertical_scroll_offset(top_of(&heights.rows, events.len()) + GAP - ui.available_height());
        }
    }
    if hide && let Some((id, offset)) = heights.restore.clone() {
        // Saved position: set the offset from the heights known so far, hidden, and re-set it until the rows stop changing; no event in the room yet -> bottom.
        match position(&id) {
            Some(at) => {
                let offset = if heights.restore_unread { -ui.available_height() / 4. } else { offset };
                area = area.vertical_scroll_offset((top_of(&heights.rows, at) + offset).max(0.));
                restoring = true;
            }
            None => {
                // Row not loaded yet: wait hidden at the top (so older pages get requested), never stick to the bottom.
                area = area.vertical_scroll_offset(0.);
                restoring = true;
                anchor_missing = true;
            }
        }
    }
    let go_bottom = heights.go_bottom;
    let now = std::time::Instant::now();
    let motion = crate::loading::motion(ui);
    let pane = crate::loading::pane(heights.fetch, events.len(), !hide);
    let paging = heights.paging.elapsed(pane == crate::loading::Pane::Rows && heights.fetch != crate::loading::Fetch::Done, now);
    let mut anchor = None;
    let mut visible = Vec::new();
    let out = area.stick_to_bottom(!restoring).show_viewport(ui, |ui, view| {
        ui.spacing_mut().item_spacing = Vec2::new(0., 0.);
        if hide {
            ui.set_invisible();
        }
        // The content ui came out wider than the viewport on a narrow column (the rows then aligned past the visible edge): lay out in the visible width.
        ui.set_max_width(ui.clip_rect().width().min(ui.available_width()));
        let mut y = TOP; // rows start below the top band: the anchor offset must be measured from the same origin as `top_of`
        let mut seen_first = false;
        if head.is_some() {
            // Few replies: no desert under them, they sit at the bottom like a chat (nothing to scroll then, so the offsets below are untouched).
            ui.add_space((view.height() - top_of(&heights.rows, events.len()) - GAP).max(0.));
        }
        // Older page on its way (or failed): ring and line in the top band, drawn again only while the band is in view.
        let band = ui.allocate_exact_size(Vec2::new(ui.available_width(), TOP), egui::Sense::hover()).0;
        if let Some(waited) = paging && !events.is_empty() && ui.is_rect_visible(band) {
            use crate::loading::{Fetch, Line};
            let (line, alpha) = if heights.fetch == Fetch::Failed { (Line::Failed, 1.) } else { let s = crate::loading::stage(waited); (Line::Busy { slow: s.slow }, s.line) };
            heights.retry |= crate::loading::status_line(ui, band, line, false, alpha, motion, t);
            if let (Line::Busy { .. }, Some(d)) = (line, crate::loading::redraw_in(waited, motion.is_none())) {
                ui.ctx().request_repaint_after(d);
            }
        }
        for (i, e) in events.iter().enumerate() {
            if let (matrix_domain::TimelineId::Server(id), false, false) = (&e.id.id, e.status, e.redacted) {
                let ph = crate::reactions::height(ui, &reactions_of(e, &acts.wishes), e.outgoing, members > 2, if pills_over_media(e) { (bubble_max(ui.available_width(), members > 2 && !e.outgoing) - 2. * PAD_X).min(320.) } else { ui.available_width() }, pills_over_media(e));
                heights.rows[i] = pill_shift(&mut heights.pills, id, ph, heights.rows[i]);
            }
            let h = heights.rows[i];
            // Space below row i: tight when the next bubble belongs to the same group.
            let gap = if glued(i + 1) { TIGHT } else { GAP };
            if jump != Some(i) && (y + h < view.min.y - 400. || y > view.max.y + 400.) {
                ui.add_space(h + gap);
            } else {
                let top = ui.cursor().top();
                if heights.divider.as_ref().is_some_and(|d| e.id.id == matrix_domain::TimelineId::Server(d.clone())) {
                    new_messages_line(ui, t);
                }
                // Sender name on the first bubble of a group; deleted rows (no bubble) keep the name rule of the history.
                let label = if e.redacted { history.sender_label(i, members) } else { (members > 2 && !e.outgoing && !glued(i)).then(|| e.sender.as_ref().map(|s| s.name.clone().unwrap_or_else(|| s.id.clone()))).flatten() };
                #[cfg(debug_assertions)]
                let hooked = head.is_none() && heights.ready && !hide && debug_press(i, events.len());
                #[cfg(debug_assertions)]
                if hooked {
                    acts.lift = Some((e.id.clone(), 0.));
                }
                let (opened, wants_save, wants_thread) = row(ui, e, pictures, avatars, acts, label.as_deref(), (glued(i), glued(i + 1)), members > 2, history.day_separator(i).as_deref(), readers.get(&i).map_or(&[][..], Vec::as_slice), t);
                #[cfg(debug_assertions)]
                let opened = if hooked { acts.held = acts.lift_rect; acts.lift_rect.map(|r| r.center()) } else { opened };
                // Touch menu: the same row once more on a layer above the dimmed chat, where the bubble already is (the menu scales that layer); taps go through it.
                if let Some(lift) = acts.lift.clone().filter(|(id, _)| *id == e.id) && let Some(rect) = acts.lift_rect {
                    let (style, line) = (ui.style().clone(), ui.max_rect());
                    egui::Area::new(crate::touch_menu::lift_layer().id).order(egui::Order::Foreground).fixed_pos(egui::pos2(line.left(), rect.top())).constrain(false).interactable(false).show(ui.ctx(), |ui| {
                        ui.set_style(style);
                        ui.set_min_width(line.width());
                        ui.set_max_width(line.width());
                        let mut inert = Acts { now_playing: acts.now_playing.clone(), lift: Some(lift), wishes: acts.wishes.clone(), ..Default::default() };
                        row(ui, e, pictures, avatars, &mut inert, label.as_deref(), (glued(i), glued(i + 1)), members > 2, None, readers.get(&i).map_or(&[][..], Vec::as_slice), t);
                    });
                }
                if let (true, matrix_domain::TimelineId::Server(id)) = (wants_thread, &e.id.id) {
                    thread = Some(id.clone());
                }
                if let Some(pos) = opened {
                    menu = Some((e.id.clone(), pos));
                }
                if let (true, matrix_domain::TimelineId::Server(id), Some(name)) = (wants_save, &e.id.id, &e.attachment_name) {
                    save = Some((id.clone(), name.clone()));
                }
                let measured = ui.cursor().top() - top;
                changed |= (measured - heights.rows[i]).abs() > 0.5;
                heights.rows[i] = measured;
                if jump == Some(i) {
                    let rect = egui::Rect::from_min_max(egui::pos2(ui.min_rect().left(), top), egui::pos2(ui.min_rect().right(), ui.cursor().top()));
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                ui.add_space(gap);
            }
            if !seen_first && y + h >= view.min.y {
                first = i;
                seen_first = true;
                anchor = Some((i, view.min.y - y));
            }
            if heights.ready && !hide && y + h > view.min.y && y + h <= view.max.y && let matrix_domain::TimelineId::Server(id) = &e.id.id {
                visible.push((i, id.clone()));
            }
            y += heights.rows[i] + gap;
        }
        ui.add_space(GAP);
        if go_bottom {
            ui.scroll_to_cursor(Some(Align::BOTTOM));
        }
    });
    heights.visible = visible;
    // Nothing to read yet: ghosts while it loads, else «empty» or the failure with its retry.
    let waited = heights.opening.elapsed(pane == crate::loading::Pane::Loading, now).unwrap_or_default();
    heights.retry |= crate::loading::pane_overlay(ui, out.inner_rect, crate::chat_background(t), pane, waited, 20., motion, t);
    if crate::loading::debug_hold(false) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    acts.view = Some(out.inner_rect);
    if heights.watching {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    }
    heights.at_bottom = out.content_size.y - (out.state.offset().y + out.inner_rect.height()) <= matrix_engine::receipts::ReadVisibility::NEAR_BOTTOM_PX;
    if hide {
        heights.passes += 1;
        heights.ready = !anchor_missing && open_done(changed, heights.at_bottom || restoring, heights.passes);
        if !heights.ready {
            if anchor_missing { heights.passes = 0; }
            ui.ctx().request_repaint();
        } else {
            heights.restore = None;
        }
    }
    if heights.ready {
        heights.pos = anchor.filter(|_| !heights.at_bottom).and_then(|(i, off)| match &events[i].id.id {
            matrix_domain::TimelineId::Server(id) => Some((id.clone(), off.max(0.))),
            _ => None,
        });
    }
    if heights.at_bottom {
        heights.go_bottom = false;
    } else if heights.ready && !events.is_empty() {
        // Round button, bottom right above the composer: more to read below. No text; the screen-reader name is the only label.
        let c = crate::dir::flip_pos(out.inner_rect, out.inner_rect.right_bottom() - Vec2::splat(36.));
        let resp = ui.interact(egui::Rect::from_center_size(c, Vec2::splat(40.)), ui.id().with("to-bottom"), egui::Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, i18n::t("Go to bottom")));
        let ink = if resp.hovered() { ui.visuals().strong_text_color() } else { rgb(t.secondary) };
        ui.painter().circle(c, 20., rgb(t.surface), egui::Stroke::new(1., rgb(t.border)));
        crate::icons::paint(ui.painter(), c, crate::icons::Icon::ArrowDown, false, crate::metrics::get().icon + 2., ink);
        if unread > 0 {
            super::list::badge(ui, c - Vec2::new(0., 26.), unread, t);
        }
        if resp.clicked() {
            heights.go_bottom = true;
            ui.ctx().request_repaint();
        }
    }
    (first, menu, save, thread)
}

#[cfg(test)]
mod open_tests {
    use super::{open_done, ScrollMemory};

    #[test]
    fn groups_break_on_sender_time_day_status_and_deleted_rows() {
        use super::{joined, GroupKey};
        const MIN: u64 = 60_000;
        // Row i is glued above / below; a day separator stands above the rows listed in `days`.
        let shape = |rows: &[GroupKey], days: &[usize]| {
            let glued = |i: usize| i > 0 && i < rows.len() && joined(rows[i - 1], rows[i], || days.contains(&i));
            (0..rows.len()).map(|i| (glued(i), glued(i + 1))).collect::<Vec<_>>()
        };
        let (first, middle, last, single) = ((false, true), (true, true), (true, false), (false, false));
        let (anna, bruno) = (Some("@a:x"), Some("@b:x"));
        // Three from Anna (the third still inside the window), another sender, an own pair, Anna again, then Anna after the window.
        let rows = [(anna, 0, true), (anna, MIN, true), (anna, 11 * MIN, true), (bruno, 11 * MIN, true), (None, 12 * MIN, true), (None, 12 * MIN, true), (anna, 13 * MIN, true), (anna, 24 * MIN, true)];
        assert_eq!(shape(&rows, &[]), [first, middle, last, single, first, last, single, single]);
        // A status or deleted row (no bubble) breaks the group on both sides.
        assert_eq!(shape(&[(anna, 0, true), (anna, 1, false), (anna, 2, true)], &[]), [single, single, single]);
        // A day separator breaks it; an incoming row without a sender never joins.
        assert_eq!(shape(&[(anna, 0, true), (anna, 1, true), (anna, 2, true)], &[1]), [single, first, last]);
        assert_eq!(shape(&[(Some(""), 0, true), (Some(""), 1, true)], &[]), [single, single]);
    }

    #[test]
    fn scroll_memory_roundtrips_and_reports_changes() {
        let mut m = ScrollMemory::default();
        let ev = |s: &str| matrix_domain::EventId::new(s);
        assert!(m.set("!a:x", Some((ev("$1"), 12.4))));
        assert!(!m.set("!a:x", Some((ev("$1"), 12.4))));
        assert!(m.set("!b:x", None));
        let back = ScrollMemory::from_json(&m.to_json());
        assert_eq!(back.get("!a:x"), Some((ev("$1"), 12.)));
        assert_eq!(back.get("!b:x"), None);
        assert_eq!(back.get("!c:x"), None);
        assert!(ScrollMemory::from_json("not json").0.is_empty());
        assert_eq!(ScrollMemory::from_json(r#"{"!a:x":{"at_bottom":false}}"#).get("!a:x"), None);
    }

    #[test]
    fn waits_for_stable_bottom() {
        assert!(!open_done(true, true, 1));
        assert!(!open_done(false, false, 1));
        assert!(open_done(false, true, 2));
        assert!(open_done(true, false, 8));
    }
}
