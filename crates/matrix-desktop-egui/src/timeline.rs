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
    pub open_link: Option<matrix_desktop::SafeWebUrl>,
    pub picture: Option<(matrix_domain::EventId, bool, String)>,
}

fn link_parts(text: &str) -> Vec<(&str, Option<matrix_desktop::SafeWebUrl>)> {
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
    #[test]
    fn links_preserve_surrounding_text_and_reject_unsafe_urls() {
        let parts = super::link_parts("Ciao https://example.org/a e javascript:alert(1)");
        assert_eq!(parts.iter().map(|(s, _)| *s).collect::<String>(), "Ciao https://example.org/a e javascript:alert(1)");
        assert_eq!(parts.iter().filter(|(_, url)| url.is_some()).count(), 1);
        assert_eq!(parts.iter().find_map(|(_, url)| url.as_ref()).unwrap().as_str(), "https://example.org/a");
    }
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
fn row(ui: &mut egui::Ui, e: &DemoEvent, pictures: &Pictures, avatars: &Avatars, acts: &mut Acts, label: Option<&str>, initials: Option<&str>, photo: Option<&str>, group: bool, day: Option<&str>, read: bool, t: &DesignTokens) -> (Option<egui::Pos2>, bool, bool) {
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
    if e.redacted {
        // Compact deleted row like the GPUI client: «Name: label … time», no bubble; right-click still opens the menu.
        let gutter = group && !e.outgoing && e.sender.is_some();
        let layout = if e.outgoing { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
        let r = ui.with_layout(layout, |ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            ui.add_space(if gutter { 52. } else { 12. });
            let small = |s: &str| RichText::new(s.to_owned()).size(12.).color(rgb(t.secondary));
            if let Some(name) = label {
                ui.label(RichText::new(matrix_desktop::row_text::redacted_name(name)).size(12.).strong().color(rgb(t.accent)));
            }
            ui.label(small(matrix_desktop::row_text::REDACTED_LABEL));
            ui.label(small(&e.time));
        });
        let pos = ui.input(|i| if i.pointer.button_clicked(egui::PointerButton::Secondary) { i.pointer.interact_pos() } else { None });
        let opened = pos.filter(|p| r.response.rect.contains(*p) && ui.is_rect_visible(r.response.rect));
        return (opened, false, false);
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
            let mut w = measure(&e.stamp_read(read), 10.);
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
                            if ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale).corner_radius(CornerRadius::same(8)).sense(egui::Sense::click())).clicked() && matches!(media.kind, matrix_engine::history::MediaKind::Image | matrix_engine::history::MediaKind::Video) {
                                acts.picture = Some((id.clone(), media.kind == matrix_engine::history::MediaKind::Video, e.attachment_name.clone().unwrap_or_else(|| "immagine".into())));
                            }
                        }
                        None => {
                            if ui.add(egui::Label::new(RichText::new(&media.label).color(rgb(t.secondary))).sense(egui::Sense::click())).clicked() && media.kind == matrix_engine::history::MediaKind::Video { acts.picture = Some((id.clone(), true, e.attachment_name.clone().unwrap_or_else(|| "video".into()))); }
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
                    let parts = link_parts(&e.text);
                    if parts.iter().any(|(_, url)| url.is_some()) {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.;
                            for (text, url) in parts {
                                if let Some(url) = url {
                                    if ui.link(RichText::new(text).size(BODY)).clicked() {
                                        acts.open_link = Some(url);
                                    }
                                } else {
                                    ui.label(RichText::new(text).size(BODY).line_height(Some(20.)));
                                }
                            }
                        });
                    } else {
                        ui.label(RichText::new(&e.text).size(BODY).line_height(Some(20.)));
                    }
                }
                if let Some(th) = &e.thread {
                    ui.horizontal(|ui| {
                        if let Some(s) = &th.latest_sender {
                            let pic = s.avatar.as_deref().and_then(|m| avatars.get(m)?.as_ref());
                            let initials: String = s.name.as_deref().unwrap_or(&s.id).trim_start_matches('@').chars().take(2).collect::<String>().to_uppercase();
                            avatar(ui, 20., pic, &initials, 9., t);
                        }
                        if ui.link(format!("{} risposte", th.num_replies)).clicked() {
                            thread = true;
                        }
                    });
                }
                ui.add_space(4.);
                ui.label(RichText::new(e.stamp_read(read)).size(10.).color(rgb(t.secondary)));
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
pub fn show(ui: &mut egui::Ui, events: &[DemoEvent], history: &matrix_desktop::DemoHistory, heights: &mut Heights, pictures: &Pictures, avatars: &Avatars, acts: &mut Acts, members: u64, peer_read: Option<usize>, t: &DesignTokens, jump: Option<usize>) -> (usize, Option<(matrix_domain::ScopedTimelineId, egui::Pos2)>, Option<(matrix_domain::EventId, String)>, Option<matrix_domain::EventId>) {
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
            if jump != Some(i) && (y + h < view.min.y - 400. || y > view.max.y + 400.) {
                ui.add_space(h + gap);
            } else {
                let top = ui.cursor().top();
                let (opened, wants_save, wants_thread) = row(ui, e, pictures, avatars, acts, history.sender_label(i, members).as_deref(), history.sender_avatar(i, members).as_ref().map(|(_, i)| i.as_str()), history.sender_avatar(i, members).and_then(|(m, _)| m), members > 2, history.day_separator(i).as_deref(), peer_read.is_some_and(|last| i <= last), t);
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
                if jump == Some(i) {
                    let rect = egui::Rect::from_min_max(egui::pos2(ui.min_rect().left(), top), egui::pos2(ui.min_rect().right(), ui.cursor().top()));
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
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
