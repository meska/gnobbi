//! «Media, files and links» of the open room: three tabs inside the info panel, which on a phone is already the whole screen.
//! The engine pages the history (`matrix_engine::shared_media`); this draws what it found and says when the list's end was reached.
use crate::list::rgb;
use eframe::egui::{self, RichText, Vec2};
use matrix_desktop::{DesignTokens, i18n};
use matrix_domain::EventId;
use matrix_engine::operations::{MoreHits, SharedItem, SharedKind, SharedState, SharedTab};

/// Columns of the media grid: three on a phone, five where the panel is wide.
pub fn columns(single: bool) -> usize {
    if single { 3 } else { 5 }
}

const GAP: f32 = 2.;

/// Side of a grid cell for `width` points and `cols` columns.
fn side(width: f32, cols: usize) -> f32 {
    ((width - GAP * (cols as f32 - 1.)) / cols as f32).floor().max(1.)
}

pub struct State {
    pub scope: matrix_domain::RoomScope,
    pub tab: SharedTab,
    pub items: Vec<SharedItem>,
    pub more: MoreHits,
    /// Pictures on screen this frame: the thumbnails the session asks for first.
    pub visible: Vec<EventId>,
    /// Scroll position of the last automatic ask for older items: a new one needs the list to be scrolled again.
    asked_at: f32,
}

impl State {
    pub fn new(scope: matrix_domain::RoomScope, tab: SharedTab) -> Self {
        Self { scope, tab, items: Vec::new(), more: MoreHits::Loading, visible: Vec::new(), asked_at: -1. }
    }

    pub fn set(&mut self, state: SharedState) {
        (self.items, self.more) = (state.items, state.more);
    }

    /// The pictures and videos in the order of the grid: what the viewer steps through.
    pub fn viewer_items(&self) -> Vec<crate::viewer::Item> {
        self.items.iter().filter(|i| matches!(i.kind, SharedKind::Image | SharedKind::Video)).map(|i| item(i)).collect()
    }
}

fn item(i: &SharedItem) -> crate::viewer::Item {
    let video = i.kind == SharedKind::Video;
    crate::viewer::Item { id: EventId::new(&i.event_id), video, name: picture_name(i) }
}

fn picture_name(i: &SharedItem) -> String {
    if i.name.is_empty() { i18n::t(if i.kind == SharedKind::Video { "video" } else { "image" }).to_owned() } else { i.name.clone() }
}

/// What the user did in the panel this frame.
#[derive(Default)]
pub struct Out {
    pub back: bool,
    pub tab: Option<SharedTab>,
    pub picture: Option<(EventId, bool, String)>,
    pub open: Option<(EventId, String)>,
    pub save: Option<(EventId, String)>,
    pub cancel: bool,
    pub link: Option<matrix_desktop::SafeWebUrl>,
    pub more: bool,
}

fn tab_label(tab: SharedTab) -> &'static str {
    match tab {
        SharedTab::Media => i18n::t("Media"),
        SharedTab::Files => i18n::t("Files"),
        SharedTab::Links => i18n::t("Links"),
    }
}

/// Sender and day under an item.
fn byline(i: &SharedItem, t: &DesignTokens) -> RichText {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let day = matrix_desktop::time_fmt::day_label_in(i.ts, now, &chrono::Local, matrix_desktop::time_fmt::active());
    RichText::new(format!("{} · {day}", crate::list::clean_single_line(&i.sender_name))).size(11.).color(rgb(t.secondary))
}

pub fn show(ui: &mut egui::Ui, st: &mut State, pictures: &crate::timeline::Pictures, transfer: Option<(EventId, u64, Option<u64>)>, cols: usize, t: &DesignTokens) -> Out {
    let mut out = Out::default();
    ui.add_space(8.);
    ui.horizontal(|ui| {
        ui.add_space(8.);
        if crate::icons::button(ui, crate::icons::Icon::CaretLeft, false, i18n::t("Back")).clicked() {
            out.back = true;
        }
        ui.add(egui::Label::new(RichText::new(i18n::t("Media, files and links")).heading()).truncate());
    });
    ui.add_space(8.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let w = (ui.available_width() - 12. - 2. * ui.spacing().item_spacing.x) / 3.;
        for tab in [SharedTab::Media, SharedTab::Files, SharedTab::Links] {
            let on = st.tab == tab;
            if ui.add(egui::Button::new(tab_label(tab)).selected(on).min_size(Vec2::new(w, crate::mobile::target(crate::metrics::button())))).clicked() && !on {
                out.tab = Some(tab);
            }
        }
    });
    ui.add_space(8.);
    st.visible.clear();
    let area = egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(2.);
        let width = ui.available_width();
        match st.tab {
            SharedTab::Media => {
                let s = side(width, cols);
                ui.spacing_mut().item_spacing = Vec2::splat(GAP);
                for row in st.items.chunks(cols) {
                    ui.horizontal(|ui| {
                        for i in row {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(s), egui::Sense::hover());
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let id = EventId::new(&i.event_id);
                            let video = (i.kind == SharedKind::Video).then(|| i.duration_ms.map(matrix_engine::history::duration_label));
                            if crate::media_cards::square_cell(ui, rect, pictures.get(&id), video.as_ref().map(Option::as_deref), &i.event_id, t) {
                                out.picture = Some((id.clone(), i.kind == SharedKind::Video, picture_name(i)));
                            }
                            st.visible.push(id);
                        }
                    });
                }
            }
            SharedTab::Files => {
                for i in &st.items {
                    let id = EventId::new(&i.event_id);
                    ui.add_space(6.);
                    ui.horizontal(|ui| {
                        ui.add_space(12.);
                        ui.with_layout(crate::dir::col(), |ui| {
                            ui.set_max_width(ui.available_width() - 12.);
                            let running = transfer.as_ref().filter(|(e, ..)| *e == id).map(|(_, got, total)| (*got, total.or(i.size)));
                            let card = crate::media_cards::file_card(ui, i.size, &i.name, running, false, t);
                            if card.open { out.open = Some((id.clone(), i.name.clone())); }
                            if card.save { out.save = Some((id.clone(), i.name.clone())); }
                            out.cancel |= card.cancel;
                            ui.label(byline(i, t));
                        });
                    });
                    ui.separator();
                }
            }
            SharedTab::Links => {
                for i in &st.items {
                    let url = i.url.as_deref().and_then(matrix_desktop::SafeWebUrl::parse);
                    let row = ui.scope(|ui| {
                        ui.add_space(6.);
                        ui.horizontal(|ui| {
                            ui.add_space(12.);
                            ui.with_layout(crate::dir::col(), |ui| {
                                ui.set_max_width(ui.available_width() - 12.);
                                ui.add(egui::Label::new(crate::list::clean_single_line(&i.name)).truncate());
                                if let Some(u) = i.url.as_deref() {
                                    let link = RichText::new(crate::list::clean_single_line(u)).color(rgb(t.accent));
                                    ui.add(egui::Label::new(if url.is_some() { link.underline() } else { link }).truncate());
                                }
                                ui.label(byline(i, t));
                            });
                        });
                        ui.add_space(6.);
                    }).response;
                    if let Some(u) = url {
                        let resp = crate::metrics::clickable(ui.interact(row.rect, ui.id().with(("shared-link", &i.event_id)), egui::Sense::click()));
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, i.url.as_deref().unwrap_or_default()));
                        if crate::touch_list::tapped(&resp) { out.link = Some(u); }
                    }
                    ui.separator();
                }
            }
        }
        ui.add_space(8.);
        match st.more {
            MoreHits::Loading => {
                ui.vertical_centered(|ui| ui.spinner());
            }
            MoreHits::Available | MoreHits::Failed => {
                let label = if st.more == MoreHits::Failed { i18n::t("Retry") } else { i18n::t("More results") };
                ui.vertical_centered(|ui| {
                    if ui.add(egui::Button::new(label).min_size(Vec2::splat(crate::mobile::target(0.)))).clicked() { out.more = true; }
                });
            }
            MoreHits::None if st.items.is_empty() => {
                ui.vertical_centered(|ui| ui.label(RichText::new(i18n::t("No results")).color(rgb(t.secondary))));
            }
            MoreHits::None => {}
        }
        ui.add_space(8.);
    });
    // The end of the list was scrolled to: another pass, once for each place the list was scrolled to.
    let offset = area.state.offset().y;
    if st.more == MoreHits::Available && offset + area.inner_rect.height() >= area.content_size.y - 2. && st.asked_at != offset {
        st.asked_at = offset;
        out.more = true;
    }
    crate::touch_list::after(ui.ctx(), &area);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(id: &str, kind: SharedKind) -> SharedItem {
        SharedItem { event_id: id.into(), kind, sender: "@a:s".into(), sender_name: "A".into(), ts: 0, name: String::new(), size: None, duration_ms: None, url: None }
    }

    fn scope() -> matrix_domain::RoomScope {
        matrix_domain::RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!r:x") }
    }

    /// Every tap on a grid of points over the panel in turn (a fresh frame sequence each): what the panel reported for each.
    fn taps(st: &mut State, cols: usize) -> Vec<Out> {
        use eframe::egui::{Event, Modifiers, PointerButton};
        let pictures = crate::timeline::Pictures::default();
        let mut found = Vec::new();
        for (x, y) in [60., 180., 300.].into_iter().flat_map(|x| (0..700).step_by(8).map(move |y| (x, y))) {
            let ctx = crate::icons::test_context();
            let pos = egui::pos2(x, y as f32);
            let mut last = Out::default();
            for step in 0..4 {
                let events = match step {
                    2 => vec![Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
                    3 => vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
                    _ => vec![],
                };
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(360., 700.))), events, ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    let out = show(ui, st, &pictures, None, cols, &matrix_desktop::LIGHT);
                    if out.picture.is_some() || out.link.is_some() || out.tab.is_some() || out.back || out.more { last = out; }
                });
                output.textures_delta.clear();
            }
            found.push(last);
        }
        found
    }

    #[test]
    fn a_tap_on_a_cell_opens_that_picture_and_a_tap_on_a_link_opens_the_link() {
        let mut st = State::new(scope(), SharedTab::Media);
        let mut clip = picture("$clip", SharedKind::Video);
        clip.duration_ms = Some(65_000);
        st.set(SharedState { items: vec![picture("$a", SharedKind::Image), clip, picture("$b", SharedKind::Image)], more: MoreHits::None });
        let opened: Vec<_> = taps(&mut st, 3).into_iter().filter_map(|o| o.picture).collect();
        assert_eq!(opened.iter().map(|(id, video, _)| (id.as_str(), *video)).collect::<std::collections::BTreeSet<_>>(), [("$a", false), ("$clip", true), ("$b", false)].into());
        // Three pictures on screen: their thumbnails are the first to be asked for.
        assert_eq!(st.visible.len(), 3);

        let mut st = State::new(scope(), SharedTab::Links);
        let mut link = picture("$l", SharedKind::Link);
        (link.name, link.url) = ("guarda https://example.org/x".into(), Some("https://example.org/x".into()));
        st.set(SharedState { items: vec![link], more: MoreHits::None });
        let urls: Vec<_> = taps(&mut st, 3).into_iter().filter_map(|o| o.link).collect();
        assert!(!urls.is_empty() && urls.iter().all(|u| u.as_str() == "https://example.org/x"));
    }

    #[test]
    fn the_grid_fills_the_width_with_square_cells() {
        for cols in [3, 5] {
            let s = side(390., cols);
            assert!(s * cols as f32 + GAP * (cols as f32 - 1.) <= 390.);
            assert!(s * cols as f32 + GAP * (cols as f32 - 1.) > 390. - cols as f32);
        }
        assert_eq!((columns(true), columns(false)), (3, 5));
    }

    #[test]
    fn the_viewer_steps_through_pictures_and_videos_in_grid_order_and_skips_the_rest() {
        let mut st = State::new(scope(), SharedTab::Media);
        st.set(SharedState { items: vec![picture("$new", SharedKind::Image), picture("$doc", SharedKind::File), picture("$clip", SharedKind::Video), picture("$old", SharedKind::Image)], more: MoreHits::None });
        let items = st.viewer_items();
        assert_eq!(items.iter().map(|i| (i.id.as_str(), i.video)).collect::<Vec<_>>(), [("$new", false), ("$clip", true), ("$old", false)]);
        assert_eq!(items[0].name, i18n::t("image"));
    }
}
