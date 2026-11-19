//! Central column: the selected room's messages, read-only. Rows have different heights, so the
//! heights seen so far are cached and off-screen rows are replaced by empty space.
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Vec2};
use matrix_desktop::{DemoEvent, DesignTokens};

/// Poll clicks of this frame: (poll start, option index).
#[derive(Default)]
pub struct Acts {
    pub vote: Option<(matrix_domain::EventId, usize)>,
    /// The play/pause button of a voice note was pressed.
    pub play: Option<matrix_domain::EventId>,
    /// Set by the app before drawing: the note being played (or loaded) and how far it is.
    pub now_playing: Option<(matrix_domain::EventId, bool, f32)>,
}

pub type Pictures = std::collections::HashMap<matrix_domain::EventId, egui::TextureHandle>;
pub type Avatars = std::collections::HashMap<String, Option<egui::TextureHandle>>;

/// Round avatar of `size`: the photo when there is one, else initials on the fallback colour.
fn avatar(ui: &mut egui::Ui, size: f32, pic: Option<&egui::TextureHandle>, initials: &str, font: f32, t: &DesignTokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    match pic {
        Some(tex) => {
            egui::Image::new(tex).fit_to_exact_size(rect.size()).corner_radius(CornerRadius::same((size / 2.) as u8)).paint_at(ui, rect);
        }
        None => {
            ui.painter().circle_filled(rect.center(), size / 2., rgb(matrix_desktop::AVATAR_FALLBACK));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials, egui::FontId::proportional(font), rgb(t.avatar_text));
        }
    }
}

/// Same as the GPUI client: gap between series / inside one, bubble padding, radius, width.
const GAP: f32 = 16.;
const TIGHT: f32 = 4.;
const PAD_X: f32 = 16.;
const PAD_Y: f32 = 12.;
const MAX_BUBBLE: f32 = 720.;
const BODY: f32 = 14.;
const GUESS: f32 = 56.;

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Measured row heights for the open room and the width they were measured at.
#[derive(Default)]
pub struct Heights {
    width: f32,
    rows: Vec<f32>,
    /// The newest message is in (or near) view: what lets a read receipt go out.
    pub at_bottom: bool,
}

/// True when the message was right-clicked (or long-pressed).
fn row(ui: &mut egui::Ui, e: &DemoEvent, pictures: &Pictures, avatars: &Avatars, acts: &mut Acts, label: Option<&str>, initials: Option<&str>, photo: Option<&str>, group: bool, day: Option<&str>, t: &DesignTokens) -> (Option<egui::Pos2>, bool, bool) {
    if let Some(day) = day {
        ui.add_space(8.);
        ui.vertical_centered(|ui| ui.label(RichText::new(day).size(11.).color(rgb(t.secondary))));
        ui.add_space(8.);
    }
    if e.status {
        // Quiet centred line: small round avatar + grey text, no bubble.
        let font = egui::FontId::proportional(11.);
        let (avail, text_w) = (ui.available_width(), ui.painter().layout_no_wrap(e.text.clone(), font, Color32::WHITE).size().x);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.;
            let pic = e.sender.as_ref().and_then(|s| s.avatar.as_deref()).and_then(|m| avatars.get(m)?.as_ref());
            let has_avatar = e.sender.is_some();
            ui.add_space(((avail - text_w - if has_avatar { 18. } else { 0. }) / 2.).max(0.));
            if let Some(s) = &e.sender {
                let initials: String = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
                avatar(ui, 12., pic, &initials, 7., t);
            }
            ui.label(RichText::new(&e.text).size(11.).color(rgb(t.secondary)));
        });
        return (None, false, false);
    }
    let max = (ui.available_width() * 0.85).clamp(120., MAX_BUBBLE);
    let layout = if e.outgoing { Layout::right_to_left(Align::TOP) } else { Layout::left_to_right(Align::TOP) };
    let mut opened = None;
    let mut save = false;
    let mut thread = false;
    ui.with_layout(layout, |ui| {
        ui.add_space(12.);
        // Group rooms: the sender's round avatar opens a series of incoming bubbles; the others keep its column.
        if group && !e.outgoing {
            let pic = photo.and_then(|m| avatars.get(m)?.as_ref());
            if let Some(initials) = initials {
                avatar(ui, 32., pic, initials, 12., t);
            } else {
                ui.allocate_exact_size(Vec2::splat(32.), egui::Sense::hover());
            }
            ui.add_space(8.);
        }
        let fill = if e.highlighted { rgb(t.mention_row) } else if e.outgoing { rgb(t.outgoing) } else { rgb(t.surface) };
        let frame = egui::Frame::NONE.fill(fill).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(PAD_X as i8, PAD_Y as i8)).show(ui, |ui| {
            // Shrink-wrap: as wide as the longest of text, sender and time (at most `max`), not the whole column.
            let wide = e.media.is_some() || e.poll.is_some() || e.thread.is_some() || e.attachment_name.is_some();
            let inner = max - 2. * PAD_X;
            let measure = |text: &str, size: f32| ui.painter().layout(text.to_owned(), egui::FontId::proportional(size), Color32::WHITE, inner).size().x;
            let mut w = measure(&matrix_desktop::bubble_stamp(&e.time, e.outgoing, e.send_state), 10.);
            w = w.max(if e.redacted { measure("Messaggio eliminato", BODY) } else { measure(&e.text, BODY) });
            if let Some(name) = label {
                w = w.max(measure(name, 12.));
            }
            ui.set_width(if wide { inner.min(320.).max(w) } else { w.min(inner) + 1. });
            ui.vertical(|ui| {
                if let Some(name) = label {
                    ui.label(RichText::new(name).size(12.).strong().color(rgb(t.accent)));
                }
                if let (Some(media), matrix_domain::TimelineId::Server(id)) = (&e.media, &e.id.id) {
                    match pictures.get(id) {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let scale = (max.min(320.) / size.x).min(1.).min(320. / size.y);
                            ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale).corner_radius(CornerRadius::same(8)));
                        }
                        None => {
                            ui.label(RichText::new(&media.label).color(rgb(t.secondary)));
                        }
                    }
                }
                if let (Some(_), matrix_domain::TimelineId::Server(id)) = (&e.audio, &e.id.id) {
                    let (playing, progress) = acts.now_playing.as_ref().filter(|(p, ..)| p == id).map_or((false, 0.), |(_, pl, pr)| (*pl, *pr));
                    ui.horizontal(|ui| {
                        if ui.button(if playing { "⏸" } else { "▶" }).clicked() {
                            acts.play = Some(id.clone());
                        }
                        ui.add(egui::ProgressBar::new(progress).desired_width(140.).desired_height(8.));
                    });
                }
                if let Some(name) = &e.attachment_name
                    && !e.redacted
                    && ui.button(format!("⬇ {name}")).clicked()
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
                        let r = ui.add_enabled(!poll.ended, egui::Button::new(poll.option_label(ix)).selected(answer.own).min_size(Vec2::new(200., 26.)));
                        let bar = egui::Rect::from_min_size(r.rect.left_bottom() - Vec2::new(0., 3.), Vec2::new(r.rect.width() * share, 3.));
                        ui.painter().rect_filled(bar, 1., rgb(t.accent));
                        if r.clicked() {
                            acts.vote = Some((id.clone(), ix));
                        }
                    }
                    ui.label(RichText::new(poll.footer()).size(11.).color(rgb(t.secondary)));
                } else if e.redacted {
                    ui.label(RichText::new("Messaggio eliminato").italics().color(rgb(t.secondary)));
                } else if !e.text.is_empty() {
                    ui.label(RichText::new(&e.text).size(BODY).line_height(Some(20.)));
                }
                if let Some(th) = &e.thread
                    && ui.link(format!("{} risposte", th.num_replies)).clicked()
                {
                    thread = true;
                }
                ui.add_space(4.);
                ui.label(RichText::new(matrix_desktop::bubble_stamp(&e.time, e.outgoing, e.send_state)).size(10.).color(rgb(t.secondary)));
            });
        });
        // Read from the input, not a widget over the bubble: a click-sensing widget on top would swallow the clicks of the link and the download button.
        let pos = ui.input(|i| if i.pointer.button_clicked(egui::PointerButton::Secondary) { i.pointer.interact_pos() } else { None });
        if let Some(pos) = pos.filter(|p| frame.response.rect.contains(*p) && ui.is_rect_visible(frame.response.rect)) {
            opened = Some(pos);
        }
    });
    (opened, save, thread)
}

/// Draws the rows in view; returns the index of the first one in view, for paging back.
pub fn show(ui: &mut egui::Ui, events: &[DemoEvent], history: &matrix_desktop::DemoHistory, heights: &mut Heights, pictures: &Pictures, avatars: &Avatars, acts: &mut Acts, members: u64, t: &DesignTokens) -> (usize, Option<(matrix_domain::ScopedTimelineId, egui::Pos2)>, Option<(matrix_domain::EventId, String)>, Option<matrix_domain::EventId>) {
    let mut menu = None;
    let mut save = None;
    let mut thread = None;
    if (heights.width - ui.available_width()).abs() > 1. {
        heights.width = ui.available_width();
        heights.rows.clear();
    }
    heights.rows.resize(events.len(), GUESS);
    let mut first = 0;
    let out = egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show_viewport(ui, |ui, view| {
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
                let (opened, wants_save, wants_thread) = row(ui, e, pictures, avatars, acts, history.sender_label(i, members).as_deref(), history.sender_avatar(i, members).as_ref().map(|(_, i)| i.as_str()), history.sender_avatar(i, members).and_then(|(m, _)| m), members > 2, history.day_separator(i).as_deref(), t);
                if let (true, matrix_domain::TimelineId::Server(id)) = (wants_thread, &e.id.id) {
                    thread = Some(id.clone());
                }
                if let Some(pos) = opened {
                    menu = Some((e.id.clone(), pos));
                }
                if let (true, matrix_domain::TimelineId::Server(id), Some(name)) = (wants_save, &e.id.id, &e.attachment_name) {
                    save = Some((id.clone(), name.clone()));
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
    heights.at_bottom = out.content_size.y - (out.state.offset().y + out.inner_rect.height()) <= matrix_engine::receipts::ReadVisibility::NEAR_BOTTOM_PX;
    (first, menu, save, thread)
}
