//! «Sticker» tab of the emoji popup: the packs of the open room, one grid per pack; a click sends.
use eframe::egui::{self, RichText};
use matrix_desktop::i18n;
use matrix_engine::stickers::{Sticker, StickerPack};

pub const TAB_ID: &str = "emoji-tab-sticker";
const CELL: f32 = 64.;

/// Lives in the session; the composer fills `open`/`chosen`, the session fills `packs`.
#[derive(Default)]
pub struct Panel {
    /// The tab was drawn this frame (the session asks for the packs and the pictures, then clears it).
    pub open: bool,
    pub chosen: Option<Sticker>,
    pub packs: Option<Result<Vec<StickerPack>, ()>>,
    pub scope: Option<matrix_domain::RoomScope>,
    pub fetched: Option<std::time::Instant>,
}

/// Every sticker url of the loaded packs (their pictures are fetched like avatars).
pub fn urls(panel: &Panel) -> impl Iterator<Item = &str> {
    let packs: &[StickerPack] = match &panel.packs {
        Some(Ok(p)) => p,
        _ => &[],
    };
    packs.iter().flat_map(|p| &p.stickers).map(|s| s.url.as_str())
}

pub fn show(ui: &mut egui::Ui, panel: &mut Panel, textures: &crate::avatar::Avatars) {
    panel.open = true;
    // The sticker's media is the pack's own, public on the homeserver even in an encrypted room.
    ui.label(RichText::new(i18n::t("Stickers are not encrypted, like an unencrypted image")).small().weak());
    let packs = match &panel.packs {
        Some(Ok(p)) if !p.is_empty() => p.clone(),
        Some(Ok(_)) => return drop(ui.label(i18n::t("No sticker packs"))),
        Some(Err(())) => return drop(ui.label(i18n::t("Stickers unavailable"))),
        None => return drop(ui.spinner()),
    };
    egui::ScrollArea::vertical().max_height(320.).max_width(8. * (CELL + 6.)).show(ui, |ui| {
        for pack in &packs {
            ui.label(RichText::new(&pack.name).strong());
            ui.horizontal_wrapped(|ui| {
                for s in &pack.stickers {
                    let tex = match textures.get(&s.url) { Some(crate::avatar::Avatar::Ready(t)) => Some(t), _ => None };
                    let r = match tex {
                        Some(t) => ui.add(egui::Button::image(egui::Image::new(t).fit_to_exact_size(egui::vec2(CELL, CELL))).frame(false)),
                        None => ui.add_sized([CELL, CELL], egui::Button::new(&s.shortcode).frame(false)),
                    };
                    if r.on_hover_text(&s.body).clicked() {
                        panel.chosen = Some(s.clone());
                        ui.close();
                    }
                }
            });
        }
    });
}
