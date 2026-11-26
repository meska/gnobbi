//! What is about to be sent as attachments: the pending files of a composer (picked, dropped or pasted), the preview card
//! above the bar with their thumbnails, and the phone's bottom sheet behind the paperclip. The caption is the composer's own
//! text; sending turns the list into one `Attachment` per file (an album when there are several).
use crate::icons::Icon;
use eframe::egui;
use matrix_desktop::i18n;
use matrix_engine::sending::{Album, Attachment, Delivery, Voice};

/// As many files as Telegram puts in one album.
pub const MAX_ITEMS: usize = 10;

pub struct Item {
    pub name: String,
    pub data: Vec<u8>,
    /// The thumbnail, decoded on first draw and kept in memory only (`Some(None)`: not a picture).
    thumb: Option<Option<egui::TextureHandle>>,
}

#[derive(Default)]
pub struct Pending {
    pub items: Vec<Item>,
    /// «Send as file»: every item goes out as an uncompressed `m.file`; otherwise pictures and videos as media.
    pub as_file: bool,
    /// A recorded voice note (one item, no card).
    pub voice: Option<Voice>,
}

impl Pending {
    pub fn single(name: String, data: Vec<u8>, voice: Option<Voice>) -> Self {
        Self { items: vec![Item { name, data, thumb: None }], as_file: false, voice }
    }

    /// Adds a file; false when the album is full.
    pub fn push(&mut self, name: String, data: Vec<u8>) -> bool {
        let room = self.items.len() < MAX_ITEMS && self.voice.is_none();
        if room {
            self.items.push(Item { name, data, thumb: None });
        }
        room
    }

    /// One attachment per file, in order; with more than one they share an album id (index and count).
    pub fn into_attachments(self) -> Vec<Attachment> {
        let count = self.items.len();
        let id = matrix_engine::sending::fresh_transaction_id().as_str().to_owned();
        let delivery = if self.as_file { Delivery::File } else { Delivery::Compressed };
        let mut voice = self.voice;
        self.items
            .into_iter()
            .enumerate()
            .map(|(i, item)| Attachment {
                filename: item.name,
                data: item.data,
                delivery: if voice.is_some() { Delivery::Original } else { delivery },
                voice: voice.take(),
                album: (count > 1).then(|| Album { id: id.clone(), index: i as u8, count: count as u8 }),
            })
            .collect()
    }

    fn has_picture(&mut self, ctx: &egui::Context) -> bool {
        (0..self.items.len()).any(|i| self.thumb(ctx, i).is_some())
    }

    fn thumb(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureHandle> {
        let item = &mut self.items[i];
        item.thumb
            .get_or_insert_with(|| matrix_desktop::composer_image::thumbnail(&item.data).map(|img| ctx.load_texture(format!("attach-{}", item.name), img, Default::default())))
            .clone()
    }
}

/// The pending files follow the room like the draft: parked when the room is left, back when it is reopened, so a
/// preview never goes out into a room other than the one it was built in. Files read in the background land in their own room.
pub struct Shelf<K>(std::collections::HashMap<K, Pending>);

impl<K> Default for Shelf<K> {
    fn default() -> Self { Self(Default::default()) }
}

impl<K: std::hash::Hash + Eq> Shelf<K> {
    /// The open room changes from `old` to `new`: `current` is parked under `old` and replaced by what `new` had parked.
    pub fn switch(&mut self, current: &mut Option<Pending>, old: Option<K>, new: Option<&K>) {
        if let (Some(old), Some(pending)) = (old, current.take()) {
            self.0.insert(old, pending);
        }
        *current = new.and_then(|k| self.0.remove(k));
    }

    /// A file for a room that is not open; false when its album is full.
    pub fn add(&mut self, room: K, name: String, data: Vec<u8>) -> bool {
        match self.0.get_mut(&room) {
            Some(pending) => pending.push(name, data),
            None => {
                self.0.insert(room, Pending::single(name, data, None));
                true
            }
        }
    }
}

/// What the card asks for this frame.
#[derive(Default)]
pub struct Acts {
    pub add: bool,
    pub remove: Option<usize>,
    pub clear: bool,
    /// A single picture to keep as a sticker instead.
    pub sticker: bool,
}

/// The preview card: title and cancel, one thumbnail per file with its own remove button, «Add», and the choice between
/// compressed pictures and plain files. No help text: only what the sending needs.
pub fn card(ui: &mut egui::Ui, pending: &mut Pending, t: &matrix_desktop::DesignTokens, sticker_ok: bool) -> Acts {
    let mut acts = Acts::default();
    let m = crate::metrics::get();
    // A thumbnail is at least twice the touch target, so its remove button can be 44 pt without covering it.
    let side = if crate::mobile::touch() { 96. } else { 72. };
    let pictures = pending.has_picture(ui.ctx());
    egui::Frame::new().fill(crate::list::rgb(t.surface)).corner_radius(10).inner_margin(8).outer_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(i18n::t("Send files")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::info::close_icon(ui, i18n::t("Cancel")).clicked() {
                    acts.clear = true;
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            for i in 0..pending.items.len() {
                let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 8., crate::list::rgb(t.selected));
                if let Some(tex) = pending.thumb(ui.ctx(), i) {
                    // Cover the square, cropping the long side.
                    let [w, h] = tex.size();
                    let (w, h) = (w as f32, h as f32);
                    let scale = (side / w).max(side / h);
                    let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(side / (w * scale), side / (h * scale)));
                    painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
                } else {
                    let font = egui::FontId::proportional(m.body);
                    painter.text(rect.center_top() + egui::vec2(0., side * 0.3), egui::Align2::CENTER_CENTER, Icon::Paperclip.ch(), egui::FontId::new(m.icon * 1.4, crate::icons::family(false)), crate::list::rgb(t.text));
                    let shown = crate::composer::fit_name(&pending.items[i].name, |s| painter.layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE).size().x <= side - 8.);
                    painter.text(rect.center_bottom() - egui::vec2(0., side * 0.2), egui::Align2::CENTER_CENTER, shown, font, crate::list::rgb(t.text));
                }
                // The remove button sits on the corner: 44 pt on touch, a small disc on desktop.
                let b = crate::mobile::target(24.);
                let close = egui::Rect::from_min_size(egui::pos2(rect.right() - b, rect.top()), egui::Vec2::splat(b));
                let hit = ui.interact(close, ui.scope_id().with(("attach-remove", i)), egui::Sense::click());
                hit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, i18n::t("Remove attachment")));
                painter.circle_filled(close.center(), 11., egui::Color32::from_black_alpha(150));
                painter.text(close.center(), egui::Align2::CENTER_CENTER, Icon::X.ch(), egui::FontId::new(m.icon * 0.7, crate::icons::family(false)), egui::Color32::WHITE);
                acts.remove = acts.remove.or(hit.clicked().then_some(i));
            }
            if pending.items.len() < MAX_ITEMS {
                let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, i18n::t("Add")));
                ui.painter().rect_stroke(rect, 8., egui::Stroke::new(1., crate::list::rgb(t.selected)), egui::StrokeKind::Inside);
                ui.painter().text(rect.center() - egui::vec2(0., 10.), egui::Align2::CENTER_CENTER, Icon::Plus.ch(), egui::FontId::new(m.icon * 1.4, crate::icons::family(false)), crate::list::rgb(t.text));
                ui.painter().text(rect.center() + egui::vec2(0., 16.), egui::Align2::CENTER_CENTER, i18n::t("Add"), egui::FontId::proportional(m.body), crate::list::rgb(t.text));
                acts.add |= resp.clicked();
            }
        });
        if pictures || pending.items.len() > 1 {
            ui.horizontal(|ui| {
                let h = crate::metrics::button();
                let compress = ui.add_sized([0., h], egui::Button::selectable(!pending.as_file, i18n::t("Compress images"))).clicked();
                let file = ui.add_sized([0., h], egui::Button::selectable(pending.as_file, i18n::t("Send as file"))).clicked();
                pending.as_file = (pending.as_file && !compress) || file;
                if pending.items.len() == 1 && pictures && sticker_ok && crate::icons::button(ui, Icon::Sticker, false, i18n::t("Save as sticker")).clicked() {
                    acts.sticker = true;
                }
            });
        }
    });
    acts
}

/// One row of the phone's attach sheet.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SheetRow {
    Media,
    File,
    Poll,
    Emoji,
}

/// The phone's attach sheet: rows of at least 48 pt from the bottom edge; a tap outside closes it. Returns the row tapped.
pub fn sheet(ctx: &egui::Context, open: &mut bool, rows: &[(SheetRow, &str)], t: &matrix_desktop::DesignTokens) -> Option<SheetRow> {
    let screen = ctx.content_rect();
    let mut chosen = None;
    let backdrop = egui::Area::new(egui::Id::new("attach-sheet-backdrop")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| ui.allocate_response(screen.size(), egui::Sense::click()));
    ctx.layer_painter(backdrop.response.layer_id).rect_filled(screen, 0., egui::Color32::from_black_alpha(90));
    let area = egui::Area::new(egui::Id::new("attach-sheet")).order(egui::Order::Tooltip).pivot(egui::Align2::CENTER_BOTTOM).fixed_pos(egui::pos2(screen.center().x, screen.bottom())).show(ctx, |ui| {
        egui::Frame::new().fill(crate::list::rgb(t.surface)).corner_radius(egui::CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }).inner_margin(8).show(ui, |ui| {
            ui.set_width(screen.width() - 16.);
            for (row, label) in rows {
                let h = crate::mobile::target(44.).max(48.);
                if ui.add_sized([ui.available_width(), h], egui::Button::new(*label).frame(false)).clicked() {
                    chosen = Some(*row);
                }
            }
        });
    });
    if chosen.is_some() || (backdrop.inner.clicked() && !area.response.contains_pointer()) {
        *open = false;
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_files_share_an_album_and_one_alone_has_none() {
        let mut p = Pending::single("a.png".into(), vec![1], None);
        assert!(p.push("b.png".into(), vec![2]));
        assert!(p.push("c.mp4".into(), vec![3]));
        let out = p.into_attachments();
        let albums: Vec<_> = out.iter().map(|a| a.album.clone().unwrap()).collect();
        assert!(albums.iter().all(|a| a.id == albums[0].id && a.count == 3));
        assert_eq!(albums.iter().map(|a| a.index).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(out.iter().all(|a| a.delivery == Delivery::Compressed));
        let one = Pending::single("a.png".into(), vec![1], None).into_attachments();
        assert!(one[0].album.is_none());
    }

    #[test]
    fn as_file_sends_every_item_as_file_and_the_album_is_capped() {
        let mut p = Pending::single("a.png".into(), vec![1], None);
        p.as_file = true;
        for i in 1..MAX_ITEMS {
            assert!(p.push(format!("{i}.png"), vec![0]));
        }
        assert!(!p.push("one-too-many.png".into(), vec![0]));
        assert!(p.into_attachments().iter().all(|a| a.delivery == Delivery::File));
    }

    #[test]
    fn a_voice_note_stays_alone_and_original() {
        let voice = Voice { duration_ms: 1, waveform: vec![0.5] };
        let mut p = Pending::single("n.wav".into(), vec![1], Some(voice));
        assert!(!p.push("x.png".into(), vec![1]));
        let out = p.into_attachments();
        assert!(out[0].voice.is_some() && out[0].album.is_none() && out[0].delivery == Delivery::Original);
    }

    #[test]
    fn the_preview_stays_in_its_room_and_a_late_file_joins_its_own() {
        let mut shelf = Shelf::default();
        let mut open = Some(Pending::single("a.png".into(), vec![1], None));
        shelf.switch(&mut open, Some("cifrata"), Some(&"chiara"));
        assert!(open.is_none(), "the other room starts with no preview");
        assert!(shelf.add("cifrata", "b.png".into(), vec![2]));
        shelf.switch(&mut open, Some("chiara"), Some(&"cifrata"));
        assert_eq!(open.unwrap().items.len(), 2, "back in the room: its files, the late one included");
        let mut none = None;
        shelf.switch(&mut none, Some("cifrata"), Some(&"cifrata"));
        assert!(none.is_none(), "a sent or removed preview is not parked again");
    }
}
