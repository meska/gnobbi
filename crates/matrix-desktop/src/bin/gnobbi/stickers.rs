//! Stickers: the «Sticker» tab of the emoji popup (packs, recent, favourites, search), the editor that turns a picture
//! into a sticker and the window that manages the packs. Packs are Matrix image packs; recent, favourites and the pack order are local.
use crate::list::rgb;
use crate::avatar::{Avatar, Avatars};
use crate::icons::Icon;
use eframe::egui::{self, Color32, RichText, Sense, Vec2};
use matrix_desktop::{DesignTokens, i18n};
use matrix_engine::stickers::{PackEdit, Prepared, Sticker, StickerPack};
use serde_json::{Value, json};

pub const TAB_ID: &str = "emoji-tab-sticker";
pub const MIN_CELL: f32 = 64.;
pub const GAP: f32 = 4.;
const RECENT: usize = 30;

/// Columns and cell side that fill `width` exactly with cells of at least `MIN_CELL` (one column when narrower).
pub fn grid(width: f32) -> (usize, f32) {
    let cols = (((width + GAP) / (MIN_CELL + GAP)).floor() as usize).max(1);
    (cols, (width - (cols - 1) as f32 * GAP) / cols as f32)
}

/// Width of the tab in a window `window` wide: what the popup's margins leave, never more than five roomy columns.
pub fn width(window: f32) -> f32 {
    (window - 40.).clamp(200., 372.)
}

/// Side of a pack-bar slot: the scale's button, never under the 44 pt touch target.
pub fn bar_slot() -> f32 {
    crate::metrics::button().max(44.)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Section {
    Recent,
    Favourites,
    Pack(String),
}

/// What stays on this device: the last stickers sent, the favourites, the order of the packs.
#[derive(Default)]
pub struct Local {
    file: Option<std::path::PathBuf>,
    pub recent: Vec<Sticker>,
    pub favourites: Vec<Sticker>,
    pub order: Vec<String>,
}

fn to_json(s: &Sticker) -> Value {
    json!({"shortcode": s.shortcode, "body": s.body, "url": s.url, "mimetype": s.mimetype, "w": s.width, "h": s.height, "size": s.size})
}

fn from_json(v: &Value) -> Option<Sticker> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let number = |k: &str| v.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    Some(Sticker { shortcode: text("shortcode")?, body: text("body")?, url: text("url").filter(|u| u.starts_with("mxc://"))?, mimetype: text("mimetype"), width: number("w"), height: number("h"), size: number("size") })
}

impl Local {
    pub fn load(file: std::path::PathBuf) -> Self {
        let v: Value = std::fs::read(&file).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
        let list = |k: &str| v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(from_json).collect::<Vec<_>>();
        let order = v.get("order").and_then(Value::as_array).into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect();
        let mut recent = list("recent");
        recent.truncate(RECENT);
        Self { file: Some(file), recent, favourites: list("favourites"), order }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let all = |l: &[Sticker]| l.iter().map(to_json).collect::<Vec<_>>();
        let _ = std::fs::write(file, json!({"recent": all(&self.recent), "favourites": all(&self.favourites), "order": self.order}).to_string());
    }

    /// `s` was sent: first of the recent ones, which stay at most `RECENT`.
    pub fn used(&mut self, s: &Sticker) {
        self.recent.retain(|r| r.url != s.url);
        self.recent.insert(0, s.clone());
        self.recent.truncate(RECENT);
        self.save();
    }

    pub fn is_favourite(&self, s: &Sticker) -> bool {
        self.favourites.iter().any(|f| f.url == s.url)
    }

    pub fn toggle_favourite(&mut self, s: &Sticker) {
        if self.is_favourite(s) { self.favourites.retain(|f| f.url != s.url) } else { self.favourites.insert(0, s.clone()) }
        self.save();
    }

    /// The packs in the user's order; those never moved keep the server's order, after the others.
    pub fn sorted(&self, packs: &[StickerPack]) -> Vec<StickerPack> {
        let mut out = packs.to_vec();
        out.sort_by_key(|p| self.order.iter().position(|id| *id == p.id).unwrap_or(usize::MAX));
        out
    }

    /// Moves pack `id` one place up or down among `packs`.
    pub fn shift(&mut self, packs: &[StickerPack], id: &str, up: bool) {
        let mut ids: Vec<String> = self.sorted(packs).into_iter().map(|p| p.id).collect();
        let Some(at) = ids.iter().position(|p| p == id) else { return };
        let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|n| *n < ids.len()) };
        if let Some(to) = to {
            ids.swap(at, to);
            self.order = ids;
            self.save();
        }
    }
}

/// A picture on its way to become a sticker.
pub struct Editor {
    size: (u32, u32),
    rgba: Vec<u8>,
    fill: bool,
    ready: Option<Result<Prepared, ()>>,
    texture: Option<egui::TextureHandle>,
    code: String,
    pack: Option<String>,
}

/// State of the «Stickers» window.
#[derive(Default)]
pub struct Manage {
    open: Option<String>,
    rename: Option<(String, String)>,
    delete: Option<String>,
    new_name: String,
    import: String,
    picked: Option<(Sticker, bool)>,
}

/// Lives in the session; the composer fills `open`/`chosen`, the session fills `packs` and sends `edits`.
#[derive(Default)]
pub struct Panel {
    /// The tab was drawn this frame (the session asks for the packs and the pictures, then clears it).
    pub open: bool,
    pub chosen: Option<Sticker>,
    pub packs: Option<Result<Vec<StickerPack>, ()>>,
    pub scope: Option<matrix_domain::RoomScope>,
    pub fetched: Option<std::time::Instant>,
    pub local: Local,
    section: Option<Section>,
    query: String,
    /// Changes asked by the user, taken by the session.
    pub edits: Vec<PackEdit>,
    pub busy: bool,
    pub error: Option<&'static str>,
    pub editor: Option<Editor>,
    pub manage: Option<Manage>,
    /// The sticker whose actions are shown, and whether its «move to» list is.
    picked: Option<(Sticker, bool)>,
}

impl Panel {
    pub fn load(file: std::path::PathBuf) -> Self {
        Self { local: Local::load(file), ..Default::default() }
    }

    fn loaded(&self) -> &[StickerPack] {
        match &self.packs {
            Some(Ok(p)) => p,
            _ => &[],
        }
    }

    /// Something on screen shows the packs: the session keeps them fresh.
    pub fn wants_packs(&mut self) -> bool {
        std::mem::take(&mut self.open) || self.manage.is_some() || self.editor.is_some()
    }

    fn edit(&mut self, edit: PackEdit) {
        self.error = None;
        self.busy = true;
        self.edits.push(edit);
    }

    /// The answer to an edit: the packs are read again either way.
    pub fn edited(&mut self, ok: bool) {
        self.busy = !self.edits.is_empty();
        self.fetched = None;
        if !ok {
            self.error = Some(i18n::t("Stickers not saved"));
        }
    }

    /// Opens the editor on an encoded picture (a file, a pasted or inserted image).
    pub fn add_picture(&mut self, data: &[u8], name: &str) {
        match matrix_engine::stickers::decode(data) {
            Ok((w, h, rgba)) => self.add_rgba(w, h, rgba, name),
            Err(_) => self.error = Some(i18n::t("Image cannot be used")),
        }
    }

    pub fn add_rgba(&mut self, w: u32, h: u32, rgba: Vec<u8>, name: &str) {
        self.error = None;
        self.editor = Some(Editor { size: (w, h), rgba, fill: false, ready: None, texture: None, code: matrix_engine::stickers::shortcode_from(name), pack: None });
    }
}

/// Every picture the tab and the window show (fetched like avatars).
pub fn urls(panel: &Panel) -> impl Iterator<Item = &str> {
    let packs = panel.loaded().iter();
    packs.clone().filter_map(|p| p.avatar.as_deref()).chain(packs.flat_map(|p| &p.stickers).chain(&panel.local.recent).chain(&panel.local.favourites).map(|s| s.url.as_str()))
}

enum Act {
    Send(Sticker),
    Favourite(Sticker),
    Edit(PackEdit),
}

#[cfg(test)]
thread_local!(static RECTS: std::cell::RefCell<(Vec<egui::Rect>, Vec<egui::Rect>)> = Default::default());

fn focus_ring(ui: &egui::Ui, rect: egui::Rect, resp: &egui::Response) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, 8., ui.visuals().selection.stroke, egui::StrokeKind::Inside);
    }
}

/// One sticker of `side`: the picture whole, never stretched; its description for the screen reader.
fn cell(ui: &mut egui::Ui, side: f32, s: &Sticker, textures: &Avatars) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    #[cfg(test)]
    RECTS.with_borrow_mut(|r| r.0.push(rect));
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(rect, 8., ui.visuals().widgets.hovered.weak_bg_fill);
    }
    focus_ring(ui, rect, &resp);
    match textures.get(&s.url) {
        Some(Avatar::Ready(tex)) => {
            let size = tex.size_vec2();
            let fit = size * ((side - 8.) / size.x.max(size.y));
            egui::Image::new(tex).paint_at(ui, egui::Rect::from_center_size(rect.center(), fit));
        }
        Some(Avatar::Missing) => {
            crate::icons::paint(ui.painter(), rect.center(), Icon::Sticker, false, side * 0.4, ui.visuals().weak_text_color());
        }
        _ => drop(ui.painter().rect_filled(rect.shrink(side * 0.2), 8., ui.visuals().widgets.inactive.weak_bg_fill)),
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &s.body));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The actions on the picked sticker (right click or long press in the tab, a tap in the window), as one row of buttons
/// in place: a menu here would be a popup inside the popup, which closes it.
fn actions(ui: &mut egui::Ui, picked: &mut Option<(Sticker, bool)>, pack: Option<&StickerPack>, own: &[(String, String)], favourite: bool, send: bool, textures: &Avatars, act: &mut Option<Act>) {
    let Some((s, moving)) = picked.as_mut() else { return };
    let own_pack = pack.filter(|p| p.own);
    let mut close = false;
    ui.horizontal(|ui| {
        ui.set_min_height(bar_slot());
        if *moving {
            for (id, name) in own.iter().filter(|(id, _)| own_pack.is_some_and(|p| p.id != *id)) {
                if ui.button(name).clicked() && let Some(p) = own_pack {
                    *act = Some(Act::Edit(PackEdit::Move { pack: p.id.clone(), shortcode: s.shortcode.clone(), to: id.clone() }));
                }
            }
        } else {
            ui.spacing_mut().item_spacing.x = 0.;
            cell(ui, bar_slot(), s, textures);
            if send && crate::icons::button(ui, Icon::PaperPlaneRight, false, i18n::t("Send")).clicked() {
                *act = Some(Act::Send(s.clone()));
            }
            if crate::icons::button(ui, Icon::Star, favourite, if favourite { i18n::t("Remove from favourites") } else { i18n::t("Add to favourites") }).clicked() {
                *act = Some(Act::Favourite(s.clone()));
            }
            if let Some(p) = own_pack {
                if own.len() > 1 && crate::icons::button(ui, Icon::FolderSimple, false, i18n::t("Move to pack")).clicked() {
                    *moving = true;
                }
                if crate::icons::button(ui, Icon::Trash, false, i18n::t("Remove from my pack")).clicked() {
                    *act = Some(Act::Edit(PackEdit::Remove { pack: p.id.clone(), shortcode: s.shortcode.clone() }));
                }
            }
        }
        ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| close = crate::info::close_icon(ui, i18n::t("Close")).clicked());
    });
    if close || act.is_some() {
        *picked = None;
    }
}

/// A slot of the pack bar: an icon or the pack's picture, marked when it is the section shown.
fn bar_tab(ui: &mut egui::Ui, selected: bool, label: &str, t: &DesignTokens, paint: impl FnOnce(&egui::Ui, egui::Rect)) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(bar_slot()), Sense::click());
    #[cfg(test)]
    RECTS.with_borrow_mut(|r| r.1.push(rect));
    if selected || resp.hovered() {
        ui.painter().rect_filled(rect.shrink(2.), 8., if selected { rgb(t.input) } else { ui.visuals().widgets.hovered.weak_bg_fill });
    }
    focus_ring(ui, rect, &resp);
    paint(ui, rect);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn pack_picture(ui: &egui::Ui, rect: egui::Rect, pack: &StickerPack, textures: &Avatars, t: &DesignTokens) {
    let url = pack.avatar.as_deref().or(pack.stickers.first().map(|s| s.url.as_str()));
    match url.and_then(|u| textures.get(u)) {
        Some(Avatar::Ready(tex)) => {
            let size = tex.size_vec2();
            let side = rect.width().min(rect.height()) - 12.;
            egui::Image::new(tex).paint_at(ui, egui::Rect::from_center_size(rect.center(), size * (side / size.x.max(size.y))));
        }
        _ => {
            let initial: String = pack.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(15.), rgb(t.secondary));
        }
    }
}

/// A pasted picture, or one inserted by the iOS keyboard, while the tab is open goes to the editor.
fn take_paste(ui: &egui::Ui, panel: &mut Panel) {
    let mut pasted = None;
    ui.input_mut(|i| i.events.retain(|e| {
        if let egui::Event::PasteImage(image) = e {
            pasted = Some(image.clone());
            false
        } else {
            true
        }
    }));
    if let Some(image) = pasted {
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
        panel.add_rgba(image.size[0] as u32, image.size[1] as u32, rgba, "");
    }
    type Inserted = (Option<egui::Id>, Vec<Result<(Vec<u8>, Option<String>), String>>);
    if let Some((_, mut items)) = ui.data_mut(|d| d.remove_temp::<Inserted>(egui::Id::new("winit_image_insertions")))
        && let Some(Ok((data, description))) = items.pop()
    {
        panel.add_picture(&data, description.as_deref().unwrap_or(""));
    }
}

fn error_line(ui: &mut egui::Ui, panel: &mut Panel) {
    if let Some(error) = panel.error {
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, error);
            if crate::info::close_icon(ui, i18n::t("Close")).clicked() {
                panel.error = None;
            }
        });
    }
}

/// The tab: search and the two actions on top, the grid of the chosen section, the bar of the packs fixed at the bottom.
pub fn show(ui: &mut egui::Ui, panel: &mut Panel, textures: &Avatars, t: &DesignTokens) {
    panel.open = true;
    take_paste(ui, panel);
    let w = width(ui.ctx().content_rect().width());
    ui.set_width(w);
    let slot = crate::metrics::button();
    ui.horizontal(|ui| {
        let field = w - 2. * slot - 2. * ui.spacing().item_spacing.x;
        crate::metrics::search(ui, [field, slot], &mut panel.query, i18n::t("Search"));
        if crate::icons::button(ui, Icon::Plus, false, i18n::t("Add sticker")).clicked() {
            crate::pick::start(ui.ctx(), i18n::t("Add sticker"), true);
        }
        if crate::icons::button(ui, Icon::GearSix, false, i18n::t("Manage stickers")).clicked() {
            panel.manage = Some(Manage::default());
            ui.close();
        }
    });
    error_line(ui, panel);
    let (cols, side) = grid(w);
    let height = 3.5 * side + 3. * GAP; // half a row shows there is more to scroll
    let packs = match &panel.packs {
        Some(Ok(p)) => panel.local.sorted(p),
        other => {
            let failed = other.is_some();
            ui.allocate_ui_with_layout(Vec2::new(w, height + GAP + bar_slot()), egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                if failed { ui.label(i18n::t("Stickers unavailable")); } else { ui.spinner(); }
            });
            return;
        }
    };
    let own: Vec<(String, String)> = packs.iter().filter(|p| p.own).map(|p| (p.id.clone(), p.name.clone())).collect();
    let query = panel.query.trim().to_lowercase();
    let known = |s: &Section| match s {
        Section::Pack(id) => packs.iter().any(|p| p.id == *id),
        _ => true,
    };
    let section = panel.section.clone().filter(known).unwrap_or_else(|| match packs.first() {
        Some(p) if panel.local.recent.is_empty() => Section::Pack(p.id.clone()),
        _ => Section::Recent,
    });
    let home = |s: &Sticker| packs.iter().find(|p| p.stickers.iter().any(|x| x.url == s.url));
    let entries: Vec<(&Sticker, Option<&StickerPack>)> = if !query.is_empty() {
        packs.iter().flat_map(|p| {
            let all = p.name.to_lowercase().contains(&query);
            let query = &query;
            p.stickers.iter().filter(move |s| all || s.shortcode.contains(query) || s.body.to_lowercase().contains(query)).map(move |s| (s, Some(p)))
        }).collect()
    } else {
        match &section {
            Section::Recent => panel.local.recent.iter().map(|s| (s, home(s))).collect(),
            Section::Favourites => panel.local.favourites.iter().map(|s| (s, home(s))).collect(),
            Section::Pack(id) => packs.iter().filter(|p| p.id == *id).flat_map(|p| p.stickers.iter().map(move |s| (s, Some(p)))).collect(),
        }
    };
    let mut act = None;
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("sticker-grid").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(GAP);
        if entries.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(side * 0.6);
                ui.label(crate::icons::rich(Icon::Sticker, false, 40.).color(rgb(t.secondary)));
                ui.label(RichText::new(if packs.is_empty() { i18n::t("No sticker packs") } else { i18n::t("No stickers") }).color(rgb(t.secondary)));
                if query.is_empty() && packs.iter().all(|p| p.stickers.is_empty()) && ui.button(crate::icons::with_label(Icon::Plus, i18n::t("Add sticker"))).clicked() {
                    crate::pick::start(ui.ctx(), i18n::t("Add sticker"), true);
                }
            });
        }
        for row in entries.chunks(cols) {
            ui.horizontal(|ui| {
                for (s, pack) in row {
                    let r = cell(ui, side, s, textures);
                    if r.clicked() {
                        act = Some(Act::Send(Sticker::clone(s)));
                    }
                    if r.secondary_clicked() {
                        pick = Some((Sticker::clone(s), false));
                    }
                }
            });
        }
    });
    ui.add_space(GAP);
    let mut picked = None;
    let chosen = pick.is_some();
    let on = pick.or(panel.picked.take());
    let (pack, favourite) = on.as_ref().map_or((None, false), |(s, _)| (home(s), panel.local.is_favourite(s)));
    drop(entries);
    panel.picked = on;
    if chosen {
        ui.ctx().request_repaint();
    }
    // The picked sticker's actions take the place of the pack bar, same height.
    actions(ui, &mut panel.picked, pack, &own, favourite, true, textures, &mut act);
    if panel.picked.is_none() && act.is_none() {
    egui::ScrollArea::horizontal().id_salt("sticker-packs").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            let lit = |s: &Section| query.is_empty() && section == *s;
            for (s, icon, label) in [(Section::Recent, Icon::Clock, i18n::t("Recent")), (Section::Favourites, Icon::Star, i18n::t("Favourites"))] {
                let on = lit(&s);
                if bar_tab(ui, on, label, t, |ui, rect| crate::icons::paint(ui.painter(), rect.center(), icon, on, crate::metrics::get().icon + 4., rgb(if on { t.accent } else { t.secondary }))) {
                    picked = Some(s);
                }
            }
            for p in &packs {
                let s = Section::Pack(p.id.clone());
                if bar_tab(ui, lit(&s), &p.name, t, |ui, rect| pack_picture(ui, rect, p, textures, t)) {
                    picked = Some(s);
                }
            }
        });
    });
    }
    if picked.is_some() {
        panel.section = picked;
        panel.query.clear();
    }
    match act {
        Some(Act::Send(s)) => {
            panel.local.used(&s);
            panel.chosen = Some(s);
            ui.close();
        }
        Some(Act::Favourite(s)) => panel.local.toggle_favourite(&s),
        Some(Act::Edit(e)) => panel.edit(e),
        None => {}
    }
}

/// Lowercase letters, digits and `_` only, as the user types.
fn clean_code(code: &mut String) {
    *code = code.chars().flat_map(char::to_lowercase).map(|c| if c == ' ' || c == '-' { '_' } else { c }).filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').take(32).collect();
}

fn header(ui: &mut egui::Ui, title: &str) -> bool {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| crate::info::close_icon(ui, i18n::t("Close")).clicked()).inner
    })
    .inner
}

fn primary(ui: &mut egui::Ui, enabled: bool, label: &str, width: f32, t: &DesignTokens) -> bool {
    let button = egui::Button::new(RichText::new(label).color(rgb(t.on_accent))).fill(rgb(t.accent));
    ui.add_enabled_ui(enabled, |ui| ui.add_sized([width, crate::metrics::button()], button)).inner.clicked()
}

fn editor(ctx: &egui::Context, panel: &mut Panel, t: &DesignTokens) {
    let own: Vec<(String, String, Vec<String>)> = panel.loaded().iter().filter(|p| p.own).map(|p| (p.id.clone(), p.name.clone(), p.stickers.iter().map(|s| s.shortcode.clone()).collect())).collect();
    let busy = panel.busy;
    let Some(e) = &mut panel.editor else { return };
    if e.ready.is_none() {
        let ready = matrix_engine::stickers::prepare(e.size.0, e.size.1, &e.rgba, e.fill).map_err(drop);
        e.texture = ready.as_ref().ok().map(|r| ctx.load_texture("sticker-editor", egui::ColorImage::from_rgba_unmultiplied([r.side as usize; 2], &r.rgba), Default::default()));
        e.ready = Some(ready);
    }
    let target = e.pack.clone().filter(|id| own.iter().any(|(o, ..)| o == id)).or_else(|| own.first().map(|(id, ..)| id.clone()));
    let taken = own.iter().any(|(id, _, codes)| Some(id) == target.as_ref() && codes.contains(&e.code));
    let (mut close, mut save) = (false, false);
    let modal = egui::Modal::new(egui::Id::new("sticker-editor")).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        let w = (ctx.content_rect().width() - 48.).clamp(200., 320.);
        ui.set_width(w);
        ui.spacing_mut().item_spacing.y = 10.;
        close = header(ui, i18n::t("New sticker"));
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(w.min(224.)), Sense::hover());
            ui.painter().rect_filled(rect, 12., rgb(t.input));
            match (&e.texture, &e.ready) {
                (Some(tex), Some(Ok(r))) => {
                    egui::Image::new(tex).paint_at(ui, rect.shrink(12.));
                    ui.label(RichText::new(format!("{0} × {0} · {1} KB", r.side, r.png.len().div_ceil(1024))).small().color(rgb(t.secondary)));
                }
                _ => { ui.colored_label(rgb(t.error), i18n::t("Image cannot be used")); }
            }
        });
        if crate::settings::switch(ui, i18n::t("Fill the square"), &mut e.fill, t) {
            e.ready = None;
        }
        ui.add(crate::metrics::line(&mut e.code).hint_text(i18n::t("Name")).desired_width(f32::INFINITY));
        clean_code(&mut e.code);
        if taken {
            ui.colored_label(rgb(t.error), i18n::t("Name already used"));
        }
        if own.len() > 1 {
            let name = own.iter().find(|(id, ..)| Some(id) == target.as_ref()).map_or("", |(_, n, _)| n.as_str());
            crate::settings::choice(ui, i18n::t("Pack"), name, |ui| {
                for (id, name, _) in &own {
                    ui.selectable_value(&mut e.pack, Some(id.clone()), name);
                }
            });
        }
        let ok = matches!(e.ready, Some(Ok(_))) && matrix_engine::stickers::valid_shortcode(&e.code) && !taken && !busy;
        save = primary(ui, ok, i18n::t("Save"), w, t);
    }));
    if save && let Some(Editor { ready: Some(Ok(ready)), code, .. }) = panel.editor.take() {
        panel.edit(PackEdit::Add { pack: target, default_name: i18n::t("Stickers").to_owned(), shortcode: code, png: ready.png, side: ready.side });
    } else if close || modal.should_close() {
        panel.editor = None;
    }
}

fn manage(ctx: &egui::Context, panel: &mut Panel, textures: &Avatars, t: &DesignTokens) {
    let Some(mut m) = panel.manage.take() else { return };
    let packs = panel.local.sorted(panel.loaded());
    let own: Vec<(String, String)> = packs.iter().filter(|p| p.own).map(|p| (p.id.clone(), p.name.clone())).collect();
    let (busy, loading) = (panel.busy, panel.packs.is_none());
    let mut act = None;
    let mut shift = None;
    let mut close = false;
    let slot = crate::metrics::button();
    let modal = egui::Modal::new(egui::Id::new("sticker-manage")).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        let w = (ctx.content_rect().width() - 48.).clamp(200., 380.);
        ui.set_width(w);
        close = header(ui, i18n::t("Stickers"));
        error_line(ui, panel);
        egui::ScrollArea::vertical().id_salt("sticker-manage-list").max_height((ctx.content_rect().height() - 5. * slot - 96.).max(2. * slot)).show(ui, |ui| {
            ui.set_width(w);
            if loading {
                ui.spinner();
            }
            for p in &packs {
                let open = m.open.as_deref() == Some(p.id.as_str());
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(slot), Sense::hover());
                    pack_picture(ui, rect, p, textures, t);
                    ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
                        let mut on = p.enabled;
                        if crate::settings::toggle(ui, &i18n::t("Show {name}").replace("{name}", &p.name), &mut on, t) && !busy {
                            act = Some(Act::Edit(PackEdit::Enable { pack: p.id.clone(), on }));
                        }
                        if crate::icons::button(ui, if open { Icon::CaretUp } else { Icon::CaretDown }, false, &p.name).clicked() {
                            m.open = (!open).then(|| p.id.clone());
                            (m.rename, m.delete, m.picked) = (None, None, None);
                        }
                        ui.label(RichText::new(p.stickers.len().to_string()).small().color(rgb(t.secondary)));
                        ui.with_layout(crate::dir::row(egui::Align::Center), |ui| ui.add(egui::Label::new(&p.name).truncate()));
                    });
                });
                if !open {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    if crate::icons::button(ui, Icon::ArrowUp, false, i18n::t("Move up")).clicked() { shift = Some((p.id.clone(), true)); }
                    if crate::icons::button(ui, Icon::ArrowDown, false, i18n::t("Move down")).clicked() { shift = Some((p.id.clone(), false)); }
                    if p.own {
                        if crate::icons::button(ui, Icon::PencilSimple, false, i18n::t("Rename")).clicked() { (m.rename, m.delete) = (Some((p.id.clone(), p.name.clone())), None); }
                        if crate::icons::button(ui, Icon::Trash, false, i18n::t("Delete pack")).clicked() { (m.delete, m.rename) = (Some(p.id.clone()), None); }
                    }
                });
                if let Some((id, name)) = m.rename.as_mut().filter(|(id, _)| *id == p.id) {
                    let mut done = false;
                    ui.horizontal(|ui| {
                        ui.add_sized([w - slot - ui.spacing().item_spacing.x, slot], crate::metrics::line(name).hint_text(i18n::t("Name")));
                        done = crate::icons::button(ui, Icon::Check, false, i18n::t("Save")).clicked() && !name.trim().is_empty() && !busy;
                    });
                    if done {
                        act = Some(Act::Edit(PackEdit::Rename { pack: id.clone(), name: name.trim().to_owned() }));
                        m.rename = None;
                    }
                }
                if m.delete.as_deref() == Some(p.id.as_str()) {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new(RichText::new(i18n::t("Confirm deletion")).color(rgb(t.error)))).clicked() {
                            act = Some(Act::Edit(PackEdit::Delete { pack: p.id.clone() }));
                            (m.delete, m.open) = (None, None);
                        }
                        if crate::info::close_icon(ui, i18n::t("Cancel")).clicked() {
                            m.delete = None;
                        }
                    });
                }
                let favourite = m.picked.as_ref().is_some_and(|(s, _)| panel.local.is_favourite(s));
                actions(ui, &mut m.picked, Some(p), &own, favourite, false, textures, &mut act);
                let (cols, side) = grid(w);
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(GAP);
                    for row in p.stickers.chunks(cols) {
                        ui.horizontal(|ui| {
                            for s in row {
                                if cell(ui, side, s, textures).clicked() {
                                    m.picked = Some((s.clone(), false));
                                }
                            }
                        });
                    }
                });
            }
        });
        ui.add_space(8.);
        let field = w - slot - ui.spacing().item_spacing.x;
        ui.horizontal(|ui| {
            ui.add_sized([field, slot], crate::metrics::line(&mut m.new_name).hint_text(i18n::t("New pack")));
            if ui.add_enabled_ui(!busy && !m.new_name.trim().is_empty(), |ui| crate::icons::button(ui, Icon::Plus, false, i18n::t("Create pack"))).inner.clicked() {
                act = Some(Act::Edit(PackEdit::Create { name: std::mem::take(&mut m.new_name).trim().to_owned() }));
            }
        });
        ui.horizontal(|ui| {
            ui.add_sized([field, slot], crate::metrics::line(&mut m.import).hint_text(i18n::t("Room address")));
            if ui.add_enabled_ui(!busy && !m.import.trim().is_empty(), |ui| crate::icons::button(ui, Icon::DownloadSimple, false, i18n::t("Import packs from room"))).inner.clicked() {
                act = Some(Act::Edit(PackEdit::Import { room: std::mem::take(&mut m.import).trim().to_owned() }));
            }
        });
        if busy {
            ui.spinner();
        }
    }));
    if let Some((id, up)) = shift {
        let loaded = panel.loaded().to_vec();
        panel.local.shift(&loaded, &id, up);
    }
    match act {
        Some(Act::Favourite(s)) => panel.local.toggle_favourite(&s),
        Some(Act::Edit(e)) => panel.edit(e),
        _ => {}
    }
    if !(close || modal.should_close()) {
        panel.manage = Some(m);
    }
}

/// The editor and the «Stickers» window, over everything; also takes the picture picked with «Add sticker».
pub fn windows(ctx: &egui::Context, panel: &mut Panel, textures: &Avatars, t: &DesignTokens) {
    if let Some(path) = crate::pick::done(i18n::t("Add sticker")) {
        let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= matrix_engine::media::MAX_MEDIA_BYTES as u64);
        match small.then(|| std::fs::read(&path).ok()).flatten() {
            Some(data) => panel.add_picture(&data, &path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            None => panel.error = Some(i18n::t("Image cannot be used")),
        }
    }
    manage(ctx, panel, textures, t);
    editor(ctx, panel, t);
    // An error with no surface of its own (a failed pick or save after the windows closed) still has to be seen.
    if panel.error.is_some() && panel.manage.is_none() && panel.editor.is_none() && !panel.open {
        egui::Area::new(egui::Id::new("sticker-error")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_BOTTOM, [0., -96.]).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| error_line(ui, panel));
        });
    }
}

/// Debug builds, GNOBBI_DEBUG_STICKER_FIXTURE=<packs>: that many packs (0 = the empty state) with painted pictures, no server.
#[cfg(debug_assertions)]
pub fn fixture(ctx: &egui::Context, panel: &mut Panel, textures: &mut Avatars) -> bool {
    let Some(count) = matrix_desktop::hooks::var("GNOBBI_DEBUG_STICKER_FIXTURE").ok().and_then(|v| v.parse::<usize>().ok()) else { return false };
    if panel.packs.is_some() {
        return true;
    }
    let names = ["Gatti", "Lavoro", "Estate", "Famiglia"];
    let mut packs = Vec::new();
    for (p, name) in names.iter().enumerate().take(count) {
        let stickers = (0..[11, 4, 7, 2][p]).map(|i| {
            let url = format!("mxc://fixture/{p}-{i}");
            // A disc, a wide bar or a tall bar on transparent: square, landscape and portrait pictures.
            let (w, h) = [(96, 96), (96, 56), (56, 96)][i % 3];
            let hue = egui::ecolor::Hsva::new((p * 5 + i) as f32 / 16., 0.65, 0.9, 1.);
            let mut image = egui::ColorImage::filled([w, h], Color32::TRANSPARENT);
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = ((x as f32 + 0.5) / w as f32 - 0.5, (y as f32 + 0.5) / h as f32 - 0.5);
                    if dx * dx + dy * dy < 0.24 {
                        image.pixels[y * w + x] = hue.into();
                    }
                }
            }
            textures.insert(url.clone(), Avatar::Ready(ctx.load_texture(&url, image, Default::default())));
            Sticker { shortcode: format!("s{i}"), body: format!("{name} {i}"), url, mimetype: Some("image/png".into()), width: Some(w as u32), height: Some(h as u32), size: None }
        }).collect();
        packs.push(StickerPack { id: format!("!fixture:x/p{p}"), name: (*name).to_owned(), avatar: None, own: p != 1, enabled: p != 3, stickers });
    }
    panel.packs = Some(Ok(packs));
    true
}

/// Debug builds, GNOBBI_EGUI_VIEW=stickers | stickers-manage | stickers-editor: the tab (in a popup frame at the bottom), the window
/// or the editor alone on the sign-in screen, fed by the fixture; for screenshots without an account.
#[cfg(debug_assertions)]
pub fn demo(ctx: &egui::Context, t: &DesignTokens) {
    thread_local!(static DEMO: std::cell::RefCell<(Panel, Avatars)> = Default::default());
    let view = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
    if !view.starts_with("stickers") {
        return;
    }
    DEMO.with_borrow_mut(|(panel, textures)| {
        if panel.packs.is_none() {
            if !fixture(ctx, panel, textures) {
                panel.packs = Some(Ok(Vec::new()));
            }
            let first = panel.loaded().first().cloned();
            match view.as_str() {
                "stickers-manage" => panel.manage = Some(Manage { open: first.map(|p| p.id), ..Default::default() }),
                "stickers-editor" => {
                    let rgba: Vec<u8> = (0..300 * 200).flat_map(|i| [(i % 300) as u8, (i / 300) as u8, 160, 255]).collect();
                    panel.add_rgba(300, 200, rgba, "Gatto felice");
                }
                _ => panel.section = first.map(|p| Section::Pack(p.id)),
            }
        }
        if view == "stickers" {
            egui::Area::new(egui::Id::new("sticker-demo")).order(egui::Order::Foreground).anchor(egui::Align2::LEFT_BOTTOM, [8., -8.]).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(rgb(t.surface)).show(ui, |ui| crate::dir::scope(ui, |ui| show(ui, panel, textures, t)));
            });
        }
        windows(ctx, panel, textures, t);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sticker(n: usize) -> Sticker {
        Sticker { shortcode: format!("s{n}"), body: format!("Sticker {n}"), url: format!("mxc://x/{n}"), mimetype: None, width: Some(64), height: None, size: None }
    }

    fn pack(id: &str) -> StickerPack {
        StickerPack { id: id.into(), name: id.to_uppercase(), avatar: None, own: true, enabled: true, stickers: (0..7).map(sticker).collect() }
    }

    #[test]
    fn grid_fills_the_width_with_touch_sized_cells_from_300_to_600() {
        for window in (300..=600).step_by(7) {
            for zoom in [1., 1.15, 1.34] {
                let w = width(window as f32 / zoom);
                let (cols, side) = grid(w);
                assert!(side >= MIN_CELL && side >= 44., "{window} {zoom}: {side}");
                assert!((cols as f32 * side + (cols - 1) as f32 * GAP - w).abs() < 0.01, "{window} {zoom}: the grid is as wide as the tab");
                assert!(w + 40. <= (window as f32 / zoom).max(240.), "{window} {zoom}: the tab fits the window");
            }
        }
        assert_eq!(grid(372.).0, 5);
        assert_eq!(grid(10.).0, 1);
    }

    /// One frame of the tab in a window `window` wide: its rect, the rects of the sticker cells and of the pack-bar slots.
    fn frame(ctx: &egui::Context, window: f32, panel: &mut Panel, events: Vec<egui::Event>) -> (egui::Rect, Vec<egui::Rect>, Vec<egui::Rect>) {
        let t = matrix_desktop::Theme::Classic.tokens();
        let mut area = egui::Rect::NOTHING;
        RECTS.take();
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(window, 700.))), events, ..Default::default() }, |ui| {
            area = ui.scope(|ui| show(ui, panel, &Avatars::new(), &t)).response.rect;
        });
        out.textures_delta.clear();
        let (cells, slots) = RECTS.take();
        (area, cells, slots)
    }

    fn layout(window: f32, panel: &mut Panel) -> (egui::Rect, Vec<egui::Rect>, Vec<egui::Rect>) {
        let ctx = crate::icons::test_context();
        (0..3).map(|_| frame(&ctx, window, panel, Vec::new())).last().unwrap()
    }

    fn tap(ctx: &egui::Context, panel: &mut Panel, pos: egui::Pos2) {
        press(ctx, panel, pos, egui::PointerButton::Primary);
    }

    fn press(ctx: &egui::Context, panel: &mut Panel, pos: egui::Pos2, which: egui::PointerButton) {
        let button = |pressed| egui::Event::PointerButton { pos, button: which, pressed, modifiers: Default::default() };
        for events in [vec![egui::Event::PointerMoved(pos)], vec![button(true)], vec![button(false)], Vec::new()] {
            frame(ctx, 390., panel, events);
        }
    }

    #[test]
    fn a_tap_sends_the_sticker_and_remembers_it_and_the_bar_changes_the_pack_shown() {
        let ctx = crate::icons::test_context();
        let mut small = pack("b");
        small.stickers.truncate(3);
        let mut panel = Panel { packs: Some(Ok(vec![pack("a"), small])), ..Default::default() };
        let (_, cells, slots) = (0..3).map(|_| frame(&ctx, 390., &mut panel, Vec::new())).last().unwrap();
        tap(&ctx, &mut panel, cells[2].center());
        assert_eq!(panel.chosen.take().map(|s| s.shortcode), Some("s2".to_owned()));
        assert_eq!(panel.local.recent.iter().map(|s| s.shortcode.as_str()).collect::<Vec<_>>(), ["s2"]);
        // With something recent the tab opens there; the fourth slot (after Recent, Favourites and pack a) is pack b.
        assert_eq!(frame(&ctx, 390., &mut panel, Vec::new()).1.len(), 1);
        tap(&ctx, &mut panel, slots[3].center());
        assert_eq!(panel.section, Some(Section::Pack("b".into())));
        let (_, cells, slots) = frame(&ctx, 390., &mut panel, Vec::new());
        assert_eq!((cells.len(), slots.len()), (3, 4));
        // A right click (a long press on touch) puts the sticker's actions where the pack bar was; choosing one brings the bar back.
        press(&ctx, &mut panel, cells[1].center(), egui::PointerButton::Secondary);
        assert_eq!(panel.picked.as_ref().map(|(s, _)| s.shortcode.as_str()), Some("s1"));
        let (_, cells, slots) = frame(&ctx, 390., &mut panel, Vec::new());
        assert_eq!((cells.len(), slots.len()), (4, 0), "the grid plus the picked sticker, no pack bar");
        assert!(panel.chosen.is_none() && cells[3].height() >= 44.);
    }

    #[test]
    fn cells_stay_inside_the_tab_and_the_pack_bar_is_fixed_with_touch_sized_slots() {
        for window in [300., 390., 600.] {
            let mut full = Panel { packs: Some(Ok(vec![pack("a"), pack("b")])), ..Default::default() };
            let mut empty = Panel { packs: Some(Ok(Vec::new())), ..Default::default() };
            let (a, cells, slots) = layout(window, &mut full);
            let (b, none, empty_slots) = layout(window, &mut empty);
            assert!((a.width() - width(window)).abs() < 1. && a.right() <= window, "{window}: {a:?}");
            assert_eq!((cells.len(), none.len(), slots.len(), empty_slots.len()), (7, 0, 4, 2), "{window}");
            assert!(cells.iter().all(|c| c.left() >= a.left() - 0.01 && c.right() <= a.right() + 0.01 && c.width() >= MIN_CELL), "{window}: {cells:?}");
            for pair in cells.windows(2) {
                assert!(pair[0].right() <= pair[1].left() + 0.01 || pair[0].bottom() <= pair[1].top() + 0.01, "{window}: cells do not overlap");
            }
            assert!(slots.iter().all(|s| s.width() >= 44. && s.height() >= 44.), "{window}: {slots:?}");
            // Same place with and without stickers, under the grid, inside the tab.
            assert!((slots[0].top() - empty_slots[0].top()).abs() < 0.5 && (a.height() - b.height()).abs() < 0.5, "{window}: {slots:?} {empty_slots:?}");
            assert!(slots[0].top() >= cells[0].bottom() && slots[0].bottom() <= a.bottom() + 0.5, "{window}: {a:?} {:?} {:?}", cells[0], slots[0]);
        }
    }

    #[test]
    fn recent_keeps_thirty_newest_first_and_favourites_toggle() {
        let dir = std::env::temp_dir().join(format!("gnobbi-stickers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("stickers.json");
        let mut local = Local::load(file.clone());
        for n in 0..40 {
            local.used(&sticker(n));
        }
        local.used(&sticker(35));
        assert_eq!((local.recent.len(), local.recent[0].shortcode.as_str(), local.recent[1].shortcode.as_str()), (30, "s35", "s39"));
        local.toggle_favourite(&sticker(1));
        local.toggle_favourite(&sticker(2));
        local.toggle_favourite(&sticker(1));
        let packs = [pack("a"), pack("b"), pack("c")];
        local.shift(&packs, "c", true);
        local.shift(&packs, "a", true); // already first
        let back = Local::load(file);
        assert_eq!((back.recent.len(), back.favourites.len(), back.is_favourite(&sticker(2)), back.is_favourite(&sticker(1))), (30, 1, true, false));
        assert_eq!(back.sorted(&packs).iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "c", "b"]);
        let _ = std::fs::write(dir.join("bad.json"), "{\"recent\": [3, {\"url\": \"https://x\"}], \"order\": 7}");
        assert!(Local::load(dir.join("bad.json")).recent.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shortcode_field_keeps_only_valid_characters() {
        let mut code = "Gatto Felice-2!è".to_owned();
        clean_code(&mut code);
        assert_eq!(code, "gatto_felice_2");
        assert!(matrix_engine::stickers::valid_shortcode(&code));
    }
}
