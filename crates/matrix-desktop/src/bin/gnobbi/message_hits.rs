//! «Messaggi»: under the chats that match the room list's search field, the messages of every room that
//! match it, as in Telegram: chat avatar and name, date, then who wrote and the text with the term marked.
use crate::list::rgb;
use eframe::egui::{self, Align2, Color32, FontId, Sense, Vec2, text::{LayoutJob, TextFormat}};
use matrix_desktop::{DesignTokens, SidebarRoom, i18n};
use matrix_engine::operations::{MessageHit, MessageSearchState, MoreHits};

const ROW: f32 = 52.;
const HEADER: f32 = 30.;

/// What the section shows for the engine's state and the text in the field.
pub(crate) enum Section<'a> {
    Hidden,
    Searching,
    Failed,
    Hits(&'a [MessageHit], MoreHits),
}

/// Nothing for a state of another term (it would flash old results while typing) or without matches.
pub(crate) fn section<'a>(state: &'a MessageSearchState, term: &str) -> Section<'a> {
    match state {
        MessageSearchState::Searching { term: t } if t == term => Section::Searching,
        MessageSearchState::Error { term: t } if t == term => Section::Failed,
        MessageSearchState::Results { term: t, hits, more, .. } if t == term && !hits.is_empty() => Section::Hits(hits, *more),
        _ => Section::Hidden,
    }
}

pub(crate) enum Out {
    Open(Box<MessageHit>),
    More,
}

/// Where the visible text starts, as in Telegram: the beginning of the excerpt while the first matched word fits
/// in `room` (a text width), else a little before that word, so the marked word is never cut off.
fn first_match(hit: &MessageHit) -> Option<&std::ops::Range<usize>> {
    hit.matches.iter().find(|r| r.start <= r.end && r.end <= hit.excerpt.len() && hit.excerpt.is_char_boundary(r.start) && hit.excerpt.is_char_boundary(r.end))
}

fn start_of(hit: &MessageHit, room: f32, measure: &dyn Fn(&str) -> f32) -> usize {
    let Some(first) = first_match(hit) else { return 0 };
    if measure(&hit.excerpt[..first.end]) <= room {
        return 0;
    }
    // About a quarter of the line stays before the word (less when the word needs the room), behind the «…».
    let keep = (room / 4.).min(room - measure(&hit.excerpt[first.clone()]));
    let mut at = first.start;
    for (i, _) in hit.excerpt[..first.start].char_indices().rev() {
        if measure(&format!("…{}", &hit.excerpt[i..first.start])) > keep {
            break;
        }
        at = i;
    }
    at
}

/// Who wrote it, then the text on one line, the matched words in the accent colour; `measure` is the width of a text.
pub(crate) fn line(hit: &MessageHit, size: f32, who: Color32, text: Color32, mark: Color32, width: f32, measure: &dyn Fn(&str) -> f32) -> LayoutJob {
    let font = FontId::proportional(size);
    let mut job = LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width(width), ..Default::default() };
    let mut add = |s: &str, color: Color32| {
        if !s.is_empty() {
            job.append(s, 0., TextFormat::simple(font.clone(), color));
        }
    };
    // The «…» the line ends with when cut takes room too: the marked word must fit before it.
    let width = width - measure("…");
    let mut sender = format!("{}: ", hit.sender_name);
    // A long name must not push the marked word out: first name only, when the word would not fit with the whole name.
    if let Some(m) = first_match(hit)
        && measure(&sender) + measure("…") + measure(&hit.excerpt[m.clone()]) > width
    {
        sender = format!("{}: ", hit.sender_name.split_whitespace().next().unwrap_or_default());
    }
    add(&sender, who);
    let start = start_of(hit, width - measure(&sender), &|s| measure(s));
    if start > 0 {
        add("…", text);
    }
    let mut at = start;
    for r in &hit.matches {
        if r.start < at || r.end > hit.excerpt.len() {
            continue;
        }
        add(&hit.excerpt[at..r.start], text);
        add(&hit.excerpt[r.clone()], mark);
        at = r.end;
    }
    add(&hit.excerpt[at..], text);
    job
}

fn label(chat: &str, hit: &MessageHit, date: Option<&str>) -> String {
    format!("{chat}, {}, {}, {}", hit.sender_name, date.unwrap_or_default(), hit.excerpt)
}

fn row(ui: &mut egui::Ui, hit: &MessageHit, room: Option<&SidebarRoom>, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
    let resp = crate::metrics::clickable(resp);
    if crate::touch_list::pressed(ui, &resp) {
        ui.painter().rect_filled(rect, 0., rgb(t.input));
    }
    let (flip, place) = (|p: egui::Pos2| crate::dir::flip_pos(rect, p), |p: egui::Pos2, g: &egui::Galley| crate::dir::place(rect, p, g.size().x));
    let (name, color, initials, avatar) = room.map_or(("", 0x808080, "", None), |r| (r.name.as_str(), r.color, r.initials.as_str(), r.info.avatar.as_deref()));
    crate::avatar::paint(ui, flip(egui::pos2(rect.left() + 36., rect.center().y)), 40., rgb(color), &crate::avatar::lookup(avatars, avatar), initials, 15., rgb(t.avatar_text));
    let text_left = rect.left() + 68.;
    let date = matrix_desktop::time_fmt::last_message_stamp(Some(hit.ts));
    let date_w = date.as_ref().map_or(0., |d| ui.painter().layout_no_wrap(d.clone(), FontId::proportional(12.), rgb(t.secondary)).size().x);
    let title = crate::list::one_line(ui, name, 15., rgb(t.text), (rect.right() - 12. - text_left - date_w - 8.).max(20.));
    ui.painter().galley(place(egui::pos2(text_left, rect.top() + 8.), &title), title, rgb(t.text));
    if let Some(d) = &date {
        ui.painter().text(flip(egui::pos2(rect.right() - 12., rect.top() + 10.)), crate::dir::align(Align2::RIGHT_TOP), d, FontId::proportional(12.), rgb(t.secondary));
    }
    let measure = |s: &str| ui.painter().layout_no_wrap(s.to_owned(), FontId::proportional(13.), Color32::WHITE).size().x;
    let job = line(hit, 13., rgb(t.text), rgb(t.secondary), rgb(t.accent), (rect.right() - 12. - text_left).max(20.), &measure);
    let body = ui.painter().layout_job(job);
    ui.painter().galley(place(egui::pos2(text_left, rect.top() + 29.), &body), body, rgb(t.secondary));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label(name, hit, date.as_deref())));
    resp
}

/// The header and the rows; the chat of each hit comes from `room`.
pub(crate) fn show<'a>(ui: &mut egui::Ui, section: &Section<'_>, room: impl Fn(&str) -> Option<&'a SidebarRoom>, avatars: &crate::avatar::Avatars, t: &DesignTokens) -> Option<Out> {
    let mut out = None;
    let (header, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER), Sense::hover());
    let title = ui.painter().layout_no_wrap(i18n::t("Messages").to_owned(), FontId::proportional(13.), rgb(t.secondary));
    let at = egui::pos2(header.left() + 16., header.center().y - title.size().y / 2.);
    ui.painter().galley(crate::dir::place(header, at, title.size().x), title, rgb(t.secondary));
    match section {
        Section::Hidden => {}
        Section::Searching => {
            ui.vertical_centered(|ui| ui.spinner());
        }
        Section::Failed => {
            ui.horizontal(|ui| {
                ui.add_space(16.);
                ui.colored_label(rgb(t.error), i18n::t("Search failed"));
            });
        }
        Section::Hits(hits, more) => {
            for hit in *hits {
                if crate::touch_list::tapped(&row(ui, hit, room(&hit.room_id), avatars, t)) {
                    out = Some(Out::Open(Box::new(hit.clone())));
                }
            }
            match more {
                MoreHits::Available | MoreHits::Failed => {
                    let text = if matches!(more, MoreHits::Failed) { i18n::t("Retry") } else { i18n::t("More results") };
                    ui.horizontal(|ui| {
                        ui.add_space(12.);
                        if ui.button(text).clicked() {
                            out = Some(Out::More);
                        }
                    });
                }
                MoreHits::Loading => {
                    ui.vertical_centered(|ui| ui.spinner());
                }
                MoreHits::None => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(sender: &str, excerpt: &str, matches: Vec<std::ops::Range<usize>>) -> MessageHit {
        MessageHit { room_id: "!r:s".into(), event_id: "$e".into(), sender: "@a:s".into(), sender_name: sender.into(), ts: 1, excerpt: excerpt.into(), matches, thread_root: None }
    }

    /// Seven points a character.
    fn chars(s: &str) -> f32 {
        s.chars().count() as f32 * 7.
    }

    fn pieces(job: &LayoutJob) -> Vec<(&str, Color32)> {
        job.sections.iter().map(|s| (&job.text[s.byte_range.start.0..s.byte_range.end.0], s.format.color)).collect()
    }

    #[test]
    fn the_matched_words_are_marked_and_the_rest_keeps_its_colour() {
        let (who, text, mark) = (Color32::RED, Color32::WHITE, Color32::BLUE);
        let job = line(&hit("Anna", "ciao mondo bello", vec![0..4, 11..16]), 13., who, text, mark, 300., &chars);
        assert_eq!(pieces(&job), [("Anna: ", who), ("ciao", mark), (" mondo ", text), ("bello", mark)]);
        let job = line(&hit("Anna", "nessun segno", vec![]), 13., who, text, mark, 300., &chars);
        assert_eq!(pieces(&job), [("Anna: ", who), ("nessun segno", text)]);
    }

    #[test]
    fn a_word_beyond_the_visible_part_starts_the_text_a_little_before_it() {
        let (who, text, mark) = (Color32::RED, Color32::WHITE, Color32::BLUE);
        let body = "uno due tre quattro cinque sei sette otto nove dieci";
        let at = body.find("nove").unwrap();
        // 150 points = 21 characters: «Anna: » and the first words fit, «nove» does not.
        let job = line(&hit("Anna", body, vec![at..at + 4]), 13., who, text, mark, 150., &chars);
        let shown = pieces(&job);
        assert_eq!(shown.iter().find(|p| p.1 == mark).unwrap().0, "nove");
        // The word is inside the width, as the screen measures it, after a «…» and a little of the words before.
        let before: String = shown.iter().take_while(|p| p.1 != mark).map(|p| p.0).collect();
        assert!(before.starts_with("Anna: …") && before.len() > "Anna: …".len(), "{before}");
        assert!(chars(&format!("{before}nove")) <= 150., "{before}");
        // Too narrow for the whole name and the word: the first name only.
        let job = line(&hit("Anna Maria Rossi", body, vec![at..at + 4]), 13., who, text, mark, 100., &chars);
        assert!(job.text.starts_with("Anna: …"), "{}", job.text);
        // Fits from the start: no «…».
        let job = line(&hit("Anna", body, vec![0..3]), 13., who, text, mark, 150., &chars);
        assert_eq!(pieces(&job)[1], ("uno", mark));
    }

    #[test]
    fn a_range_outside_the_excerpt_is_ignored_not_a_panic() {
        let job = line(&hit("Anna", "ciao", vec![2..40, 0..2]), 13., Color32::RED, Color32::WHITE, Color32::BLUE, 300., &chars);
        assert_eq!(job.text, "Anna: ciao");
    }

    #[test]
    fn only_the_answer_to_the_text_in_the_field_shows() {
        let results = |term: &str, n: usize| MessageSearchState::Results { term: term.into(), source: matrix_engine::operations::MessageSource::LocalCache, hits: vec![hit("A", "x", vec![]); n], more: MoreHits::None, partial: false };
        assert!(matches!(section(&results("ciao", 2), "ciao"), Section::Hits(h, _) if h.len() == 2));
        assert!(matches!(section(&results("cia", 2), "ciao"), Section::Hidden));
        assert!(matches!(section(&results("ciao", 0), "ciao"), Section::Hidden));
        assert!(matches!(section(&MessageSearchState::Searching { term: "ciao".into() }, "ciao"), Section::Searching));
        assert!(matches!(section(&MessageSearchState::Error { term: "ciao".into() }, "ciao"), Section::Failed));
        assert!(matches!(section(&MessageSearchState::NoResults { term: "ciao".into(), source: matrix_engine::operations::MessageSource::LocalCache }, "ciao"), Section::Hidden));
        assert!(matches!(section(&MessageSearchState::Idle, "ciao"), Section::Hidden));
    }

    #[test]
    fn the_screen_reader_reads_chat_sender_date_and_text() {
        assert_eq!(label("Famiglia", &hit("Anna", "ciao a tutti", vec![]), Some("Lun")), "Famiglia, Anna, Lun, ciao a tutti");
    }

    #[test]
    fn hit_row_opens_on_tap_and_not_on_scroll_or_catch() {
        let (h, t) = (hit("Anna", "ciao", vec![0..4]), matrix_desktop::Theme::Dark.tokens());
        let mk = || {
            let (h, t) = (h.clone(), t);
            crate::touch_list::pad::Pad::new(move |ui, range, seen| {
                for _ in range {
                    let r = row(ui, &h, None, &Default::default(), &t);
                    seen.tapped.set(seen.tapped.get() | crate::touch_list::tapped(&r));
                }
            })
        };
        let p = egui::pos2(100., 300.);
        let mut pad = mk();
        pad.down(p);
        pad.drag(p, p - egui::vec2(0., 20.), 4);
        pad.up(p - egui::vec2(0., 20.));
        assert!(!pad.seen.tapped.get(), "a scroll");
        let mut pad = mk();
        pad.fling();
        pad.down(egui::pos2(100., 100.));
        pad.up(egui::pos2(100., 100.));
        assert!(!pad.seen.tapped.get(), "a catch");
        let mut pad = mk();
        pad.down(egui::pos2(100., 100.));
        pad.up(egui::pos2(100., 100.));
        assert!(pad.seen.tapped.get(), "a tap");
    }

    #[test]
    fn the_section_has_a_header_and_a_row_per_hit_and_a_more_button() {
        let ctx = crate::icons::test_context();
        let t = matrix_desktop::Theme::Dark.tokens();
        let hits = vec![hit("Anna", "ciao mondo", vec![0..4]), hit("Luca", "altro ciao", vec![6..10])];
        let found = Section::Hits(&hits, MoreHits::Available);
        let mut height = 0.;
        let mut out = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let start = ui.cursor().top();
            out = show(ui, &found, |_| None, &Default::default(), &t);
            height = ui.cursor().top() - start;
        });
        output.textures_delta.clear();
        assert!(out.is_none());
        assert!(height >= HEADER + 2. * ROW, "{height}");
    }
}
