//! What an attachment looks like in the chat: the album grid, the size of a single picture, the file card and the card of an
//! upload on its way. Drawn with egui shapes only (no new icon, no image asset); sizes come from the sender's declared
//! dimensions so a row keeps its height when the picture arrives.
use crate::list::rgb;
use crate::timeline::{Acts, Pictures};
use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Vec2, pos2, vec2};
use matrix_desktop::{DemoEvent, DesignTokens, i18n};
use matrix_domain::TimelineId;
use matrix_engine::history::{MediaKind, human_size};

/// What a row is in an album: one of the rows of an album run is drawn once, on the last of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Single,
    /// Not drawn: the leader shows it.
    Hidden,
    /// Draws the whole run, which starts at this row index.
    Leader(usize),
}

/// The album a picture or video belongs to, with who sent it: consecutive rows with the same key are one album (file cards never join one).
fn album_key(e: &DemoEvent) -> Option<(bool, &str, &str)> {
    let media = e.media.as_ref()?;
    let album = media.album.as_ref()?;
    if e.status || e.redacted || !matches!(e.id.id, TimelineId::Server(_)) || !matches!(media.kind, MediaKind::Image | MediaKind::Video) {
        return None;
    }
    Some((e.outgoing, e.sender.as_ref().map_or("", |s| s.id.as_str()), album.id.as_str()))
}

/// A row that says something of its own: a caption, reactions or a thread.
fn carries_content(e: &DemoEvent) -> bool {
    !e.text.is_empty() || !e.reactions.is_empty() || e.thread.is_some()
}

/// The role of every row. A run of one (the rest of its album is missing or not loaded yet) stays a single row.
pub fn album_roles(events: &[DemoEvent]) -> Vec<Role> {
    let mut roles = vec![Role::Single; events.len()];
    let mut start = 0;
    while start < events.len() {
        let key = album_key(&events[start]);
        let mut end = start + 1;
        // The mark is the sender's word: a run holds at most `count` (and 10) items, with growing indexes, and a row that
        // carries text, reactions or a thread ends it, so such a row is always drawn (as the leader), never hidden.
        let album = |e: &DemoEvent| e.media.as_ref().and_then(|m| m.album.as_ref()).map(|a| (a.index, a.count));
        let cap = album(&events[start]).map_or(1, |(_, count)| usize::from(count).clamp(1, 10));
        while key.is_some()
            && end < events.len()
            && end - start < cap
            && album_key(&events[end]) == key
            && album(&events[end]).map(|a| a.0) > album(&events[end - 1]).map(|a| a.0)
            && !carries_content(&events[end - 1])
        {
            end += 1;
        }
        if end - start > 1 {
            roles[start..end - 1].fill(Role::Hidden);
            roles[end - 1] = Role::Leader(start);
        }
        start = end;
    }
    roles
}

/// Rows of an album of `n` pictures, by how many cells each holds; two landscape pictures stack, anything else sits side by side.
fn row_pattern(aspects: &[f32]) -> Vec<usize> {
    match aspects.len() {
        0 => vec![],
        1 => vec![1],
        2 if aspects.iter().all(|a| *a > 1.2) => vec![1, 1],
        2 => vec![2],
        3 => vec![1, 2],
        4 => vec![2, 2],
        5 => vec![2, 3],
        6 => vec![3, 3],
        7 => vec![1, 3, 3],
        8 => vec![2, 3, 3],
        9 => vec![3, 3, 3],
        _ => vec![1, 3, 3, 3],
    }
}

/// Cells (relative to the grid's top-left) of an album whose pictures have these width/height ratios, in a column `width` wide,
/// and the grid's height. Every row spans the full width; a cell crops its picture to fit.
pub fn grid(aspects: &[f32], width: f32, gap: f32) -> (Vec<Rect>, f32) {
    let aspects: Vec<f32> = aspects.iter().map(|a| if a.is_finite() { a.clamp(0.66, 1.8) } else { 1. }).collect();
    let (mut cells, mut y, mut at) = (Vec::with_capacity(aspects.len()), 0., 0);
    for (r, count) in row_pattern(&aspects).into_iter().enumerate() {
        let row = &aspects[at..at + count];
        let free = width - gap * (count as f32 - 1.);
        // Shared height of the row, kept in a sane range.
        let h = (free / row.iter().sum::<f32>()).clamp(60., if count == 1 { width * 0.8 } else { width * 0.55 });
        let sum: f32 = row.iter().sum();
        let mut x = 0.;
        for a in row {
            let w = free * a / sum;
            cells.push(Rect::from_min_size(pos2(x, y), vec2(w, h)));
            x += w + gap;
        }
        y += h + if r + 1 < row_pattern(&aspects).len() { gap } else { 0. };
        at += count;
    }
    (cells, y)
}

/// The part of a `tex` x `tex` picture a cell of `cell` shape shows: the centre, as the cell is filled (uv rectangle).
fn cover(tex: Vec2, cell: Vec2) -> Rect {
    let (ta, ca) = (tex.x / tex.y, cell.x / cell.y);
    let full = Rect::from_min_max(pos2(0., 0.), pos2(1., 1.));
    if !ta.is_finite() || !ca.is_finite() || ta <= 0. || ca <= 0. {
        full
    } else if ta > ca {
        let w = ca / ta;
        Rect::from_min_max(pos2((1. - w) / 2., 0.), pos2((1. + w) / 2., 1.))
    } else {
        let h = ta / ca;
        Rect::from_min_max(pos2(0., (1. - h) / 2.), pos2(1., (1. + h) / 2.))
    }
}

/// Ratio of a picture event: the sender's declared size, else the loaded thumbnail, else a square.
fn aspect_of(e: &DemoEvent, pictures: &Pictures) -> f32 {
    let from = |w: f32, h: f32| (h > 0.).then(|| w / h);
    e.media.as_ref().and_then(|m| m.dims).and_then(|(w, h)| from(w as f32, h as f32))
        .or_else(|| match &e.id.id {
            TimelineId::Server(id) => pictures.get(id).and_then(|t| from(t.size_vec2().x, t.size_vec2().y)),
            _ => None,
        })
        .unwrap_or(1.)
}

/// The play disc over a video's picture, and its length in a small pill.
fn play_badge(ui: &egui::Ui, rect: Rect, duration: Option<&str>) {
    let p = ui.painter();
    let c = rect.center();
    p.circle_filled(c, 20., Color32::from_black_alpha(140));
    crate::icons::paint(p, c, crate::icons::Icon::Play, true, 20., Color32::WHITE);
    if let Some(d) = duration {
        let galley = p.layout_no_wrap(d.to_owned(), egui::FontId::proportional(11.), Color32::WHITE);
        let pill = Rect::from_min_size(rect.left_bottom() + vec2(6., -galley.size().y - 10.), galley.size() + vec2(10., 4.));
        p.rect_filled(pill, 8., Color32::from_black_alpha(140));
        p.galley(pill.min + vec2(5., 2.), galley, Color32::WHITE);
    }
}

/// One picture (or video) cell: its thumbnail cropped to the cell, or an empty tile while it loads. A click opens the viewer on it.
fn cell(ui: &mut egui::Ui, rect: Rect, e: &DemoEvent, pictures: &Pictures, acts: &mut Acts, t: &DesignTokens) {
    let TimelineId::Server(id) = &e.id.id else { return };
    let video = e.media.as_ref().is_some_and(|m| m.kind == MediaKind::Video);
    let round = CornerRadius::same(6);
    match pictures.get(id) {
        Some(tex) => {
            egui::Image::new(tex).uv(cover(tex.size_vec2(), rect.size())).fit_to_exact_size(rect.size()).corner_radius(round).paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, round, rgb(t.surface).gamma_multiply(0.8));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    if video {
        play_badge(ui, rect, e.video.as_ref().and_then(|v| v.duration.as_deref()));
    }
    let resp = crate::metrics::clickable(ui.interact(rect, ui.id().with(("album-cell", id.as_str())), egui::Sense::click()));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, i18n::t(if video { "Video" } else { "Image" })));
    if resp.clicked() {
        acts.picture = Some((id.clone(), video, e.attachment_name.clone().unwrap_or_else(|| i18n::t(if video { "video" } else { "image" }).into())));
    }
    // Drawn last, so that a press on it is the pill's and not the cell's.
    if pictures.get(id).is_none() || video {
        tile_pill(ui, rect, e, acts);
    }
}

/// One square cell of the shared-media grid: the thumbnail cropped to the centre (an empty tile while it loads), the play disc on a
/// video. True when it was clicked.
pub fn square_cell(ui: &mut egui::Ui, rect: Rect, tex: Option<&egui::TextureHandle>, video: Option<Option<&str>>, key: &str, t: &DesignTokens) -> bool {
    let round = CornerRadius::same(6);
    match tex {
        Some(tex) => {
            egui::Image::new(tex).uv(cover(tex.size_vec2(), rect.size())).fit_to_exact_size(rect.size()).corner_radius(round).paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, round, rgb(t.surface).gamma_multiply(0.8));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    if let Some(duration) = video {
        play_badge(ui, rect, duration);
    }
    let resp = crate::metrics::clickable(ui.interact(rect, ui.id().with(("shared-cell", key)), egui::Sense::click()));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, i18n::t(if video.is_some() { "Video" } else { "Image" })));
    crate::touch_list::tapped(&resp)
}

/// The album: its pictures and videos in a grid of `width`. Files of an album (sent with «Send as file») stay one card each.
pub fn album(ui: &mut egui::Ui, items: &[DemoEvent], pictures: &Pictures, width: f32, acts: &mut Acts, t: &DesignTokens) {
    let aspects: Vec<f32> = items.iter().map(|e| aspect_of(e, pictures)).collect();
    let (cells, height) = grid(&aspects, width, 2.);
    let (outer, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    for (e, cell_rect) in items.iter().zip(cells) {
        cell(ui, cell_rect.translate(outer.min.to_vec2()), e, pictures, acts, t);
    }
}

/// Size a single picture takes: its loaded thumbnail (never enlarged), else its declared shape in a 4:3 box, so the row does
/// not change height when the picture arrives.
pub fn single_size(dims: Option<(u32, u32)>, loaded: Option<Vec2>, avail: f32, side: f32) -> Vec2 {
    match (loaded, dims) {
        (Some(tex), _) => tex * (avail.min(side) / tex.x).min(1.).min(side / tex.y),
        (None, Some((w, h))) if w > 0 && h > 0 => {
            let shape = vec2(w as f32, h as f32);
            shape * (avail.min(side) / shape.x).min(side * 0.75 / shape.y)
        }
        _ => vec2(avail.min(side), side * 0.5),
    }
}

/// Play disc of a single video over its picture.
pub fn video_badge(ui: &egui::Ui, rect: Rect, e: &DemoEvent) {
    play_badge(ui, rect, e.video.as_ref().and_then(|v| v.duration.as_deref()));
}

/// An empty tile of the shape of a picture still to come.
pub fn tile(ui: &mut egui::Ui, size: Vec2, t: &DesignTokens) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, 8., rgb(t.surface).gamma_multiply(0.8));
    resp
}

/// What the download pill of a tile asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pill {
    Idle,
    Start,
    Cancel,
}

/// The download button on the corner of a picture or video tile: the arrow and the size («↓ 8.2 MB»), or, while it runs, the bytes
/// received over the total with a bar and a cross that cancels. `running` is `(received, total)` of this very file.
pub fn download_pill(ui: &mut egui::Ui, tile: Rect, size: Option<u64>, running: Option<(u64, Option<u64>)>, key: &str) -> Pill {
    let label = match running {
        Some((got, total)) => progress_text(got, total),
        None => size.map_or_else(String::new, human_size),
    };
    let galley = ui.painter().layout_no_wrap(label, egui::FontId::proportional(12.), Color32::WHITE);
    let (icon, pad, height) = (14., 8., 26.);
    let width = if galley.size().x > 0. { pad + icon + 4. + galley.size().x + pad } else { pad + icon + pad };
    let pill = Rect::from_min_size(tile.min + vec2(8., 11.), vec2(width.min((tile.width() - 16.).max(height)), height));
    // Pressable over at least 44 pt, whatever the pill's own size.
    let hit = Rect::from_center_size(pill.center(), vec2(pill.width().max(44.), 44.));
    let resp = crate::metrics::clickable(ui.interact(hit, ui.id().with(("download-pill", key)), egui::Sense::click()));
    let name = format!("{} {}", i18n::t(if running.is_some() { "Cancel" } else { "Download" }), galley.text());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.trim()));
    let p = ui.painter();
    p.rect_filled(pill, height / 2., Color32::from_black_alpha(if resp.hovered() { 190 } else { 150 }));
    let icon_at = pos2(pill.left() + pad + icon / 2., pill.center().y);
    crate::icons::paint(p, icon_at, if running.is_some() { crate::icons::Icon::X } else { crate::icons::Icon::DownloadSimple }, false, icon, Color32::WHITE);
    if galley.size().x > 0. {
        let at = pos2(pill.left() + pad + icon + 4., pill.center().y - galley.size().y / 2.);
        p.galley(at, galley, Color32::WHITE);
    }
    if let Some(done) = running.and_then(|(got, total)| fraction(got, total)) {
        p.rect_filled(Rect::from_min_size(pill.left_bottom() - vec2(0., 3.), vec2(pill.width() * done, 3.)), 1.5, Color32::WHITE);
    }
    match (resp.clicked(), running.is_some()) {
        (true, true) => Pill::Cancel,
        (true, false) => Pill::Start,
        _ => Pill::Idle,
    }
}

/// The pill of `e`'s tile, from what the app says: the button while it is held back, the progress while it comes. Sets what was asked.
pub fn tile_pill(ui: &mut egui::Ui, tile: Rect, e: &DemoEvent, acts: &mut Acts) {
    let (Some(auto), TimelineId::Server(id), Some(media)) = (&acts.auto, &e.id.id, &e.media) else { return };
    let running = acts.transfer.as_ref().filter(|(t, ..)| t == id).map(|(_, got, total)| (*got, total.or(e.attachment_size)));
    let Some(size) = auto.button(e).or(running.map(|_| e.attachment_size)) else { return };
    match download_pill(ui, tile, size, running, id.as_str()) {
        Pill::Start => acts.fetch = matrix_desktop::auto_download::Kind::of(media.kind).map(|k| (id.clone(), k)),
        Pill::Cancel => acts.cancel_transfer = true,
        Pill::Idle => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family { Document, Sheet, Archive, Picture, Audio, Video, Other }

fn family(name: &str) -> (Family, String) {
    let ext = name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase());
    let f = match ext.as_str() {
        "pdf" | "doc" | "docx" | "odt" | "txt" | "rtf" | "md" | "pages" => Family::Document,
        "xls" | "xlsx" | "csv" | "ods" | "numbers" => Family::Sheet,
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "bz2" => Family::Archive,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" => Family::Picture,
        "mp3" | "ogg" | "m4a" | "wav" | "flac" | "opus" => Family::Audio,
        "mp4" | "mov" | "mkv" | "webm" | "avi" => Family::Video,
        _ => Family::Other,
    };
    (f, ext.chars().take(4).collect::<String>().to_uppercase())
}

/// A page with a folded corner and the extension on it; the colour tells the family of file.
pub fn file_glyph(ui: &egui::Ui, rect: Rect, name: &str, t: &DesignTokens) {
    let (family, ext) = family(name);
    let ink = match family {
        Family::Document => Color32::from_rgb(0xd6, 0x4f, 0x4f),
        Family::Sheet => Color32::from_rgb(0x2f, 0x9e, 0x62),
        Family::Archive => Color32::from_rgb(0xd0, 0x8a, 0x1c),
        Family::Picture => Color32::from_rgb(0x8a, 0x5c, 0xd6),
        Family::Audio => Color32::from_rgb(0xd6, 0x5c, 0xa8),
        Family::Video => Color32::from_rgb(0x3c, 0x8a, 0xd6),
        Family::Other => rgb(t.accent),
    };
    let p = ui.painter();
    let fold = rect.width() * 0.3;
    let (l, r, top, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    p.add(egui::Shape::convex_polygon(vec![pos2(l, top), pos2(r - fold, top), pos2(r, top + fold), pos2(r, b), pos2(l, b)], ink, egui::Stroke::NONE));
    p.add(egui::Shape::convex_polygon(vec![pos2(r - fold, top), pos2(r, top + fold), pos2(r - fold, top + fold)], Color32::from_white_alpha(70), egui::Stroke::NONE));
    if !ext.is_empty() {
        p.text(pos2(rect.center().x, rect.bottom() - rect.height() * 0.3), egui::Align2::CENTER_CENTER, ext, egui::FontId::proportional(rect.width() * 0.28), Color32::WHITE);
    }
}

/// Bytes as «received / total» (just «received» when the total is not known).
pub fn progress_text(received: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!("{} / {}", human_size(received), human_size(total)),
        None => human_size(received),
    }
}

/// Fraction done, none while the total is unknown.
pub fn fraction(received: u64, total: Option<u64>) -> Option<f32> {
    total.filter(|t| *t > 0).map(|t| (received as f32 / t as f32).clamp(0., 1.))
}

/// Fraction of an upload; none (indeterminate) until the first byte is reported or while the total is unknown.
pub fn upload_fraction(sent: u64, total: Option<u64>) -> Option<f32> {
    if sent == 0 { None } else { fraction(sent, total) }
}

/// The part of the bar `(from, to)`, as fractions of its width, that the indeterminate segment covers at `time` seconds:
/// a segment of 30 % running to and fro, one trip every 1.6 s.
pub fn indeterminate_span(time: f64) -> (f32, f32) {
    const WIDTH: f32 = 0.3;
    const PERIOD: f64 = 1.6;
    let phase = (time.rem_euclid(PERIOD) / PERIOD) as f32;
    let from = (1. - (2. * phase - 1.).abs()) * (1. - WIDTH);
    (from, from + WIDTH)
}

fn indeterminate_bar(ui: &mut egui::Ui, fill: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.), egui::Sense::hover());
    let (from, to) = indeterminate_span(ui.input(|i| i.time));
    let radius = CornerRadius::same(3);
    ui.painter().rect_filled(rect, radius, ui.visuals().extreme_bg_color);
    let at = |f: f32| rect.left() + rect.width() * f;
    ui.painter().rect_filled(Rect::from_min_max(pos2(at(from), rect.top()), pos2(at(to), rect.bottom())), radius, fill);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// What the file card asked for this frame.
#[derive(Default)]
pub struct CardOut {
    pub fetch: bool,
    pub open: bool,
    pub reveal: bool,
    pub save: bool,
    pub cancel: bool,
}

/// The card of a file attachment: glyph, name, size or download progress, and Open / Save (Cancel while it downloads).
/// `transfer` is `(received, declared total)` of the download of this very file now running. `held` is `Some(size)` while the file is
/// not in memory and the settings keep the app from fetching it: the card then offers the download button with the size, and Open and
/// Save once it is here.
pub fn file_card(ui: &mut egui::Ui, size: Option<u64>, name: &str, transfer: Option<(u64, Option<u64>)>, held: Option<Option<u64>>, saved: bool, t: &DesignTokens) -> CardOut {
    let mut out = CardOut::default();
    let button = crate::mobile::target(0.);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.;
        let (glyph, _) = ui.allocate_exact_size(vec2(36., 44.), egui::Sense::hover());
        file_glyph(ui, glyph, name, t);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.;
            ui.add(egui::Label::new(RichText::new(crate::list::clean_single_line(name)).strong()).truncate());
            match transfer {
                Some((got, total)) => {
                    let bar = egui::ProgressBar::new(fraction(got, total).unwrap_or(0.)).desired_height(6.).animate(total.is_none()).fill(rgb(t.accent));
                    ui.add(bar);
                    ui.label(RichText::new(progress_text(got, total)).size(11.).color(rgb(t.secondary)));
                }
                None => {
                    ui.label(RichText::new(size.map_or_else(String::new, human_size)).size(11.).color(rgb(t.secondary)));
                }
            }
        });
    });
    ui.add_space(6.);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.;
        let size = Vec2::splat(button);
        if transfer.is_some() {
            out.cancel = ui.add(egui::Button::new(i18n::t("Cancel")).min_size(size)).clicked();
        } else {
            if let Some(declared) = held {
                let label = declared.map_or_else(|| i18n::t("Download").to_owned(), human_size);
                out.fetch = ui.add(egui::Button::new(crate::icons::with_label(crate::icons::Icon::DownloadSimple, &label)).min_size(size)).clicked();
                return;
            }
            // Only what the system may open without running anything; the rest is saved.
            if matrix_engine::video_file::openable_name(name).is_some() {
                out.open = ui.add(egui::Button::new(i18n::t(if cfg!(any(target_os = "ios", target_os = "android")) { "Share" } else { "Open" })).min_size(size)).clicked();
            }
            out.save = ui.add(egui::Button::new(crate::icons::with_label(crate::icons::Icon::DownloadSimple, i18n::t("Save"))).min_size(size)).clicked();
            if saved {
                out.reveal = ui.add(egui::Button::new(i18n::t("Show in folder")).min_size(size)).clicked();
            }
        }
    });
    out
}

/// The card of an attachment going out: glyph, name, bar and a cancel button. True when Cancel was pressed.
pub fn upload_card(ui: &mut egui::Ui, name: &str, sent: u64, total: u64, t: &DesignTokens) -> bool {
    let mut cancel = false;
    egui::Frame::NONE.fill(rgb(t.surface)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.;
            let (glyph, _) = ui.allocate_exact_size(vec2(28., 34.), egui::Sense::hover());
            file_glyph(ui, glyph, name, t);
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - crate::metrics::button() - 10.).max(60.));
                ui.spacing_mut().item_spacing.y = 2.;
                ui.add(egui::Label::new(crate::list::clean_single_line(name)).truncate());
                let total = (total > 0).then_some(total);
                // No byte reported yet: the file is being stored (and encrypted, in a room that is) before it goes out, so the card says it is busy, not «0 B».
                match upload_fraction(sent, total) {
                    Some(f) => {
                        ui.add(egui::ProgressBar::new(f).desired_height(6.).fill(rgb(t.accent)));
                        ui.label(RichText::new(progress_text(sent, total)).size(11.).color(rgb(t.secondary)));
                    }
                    None => {
                        indeterminate_bar(ui, rgb(t.accent));
                        ui.label(RichText::new(total.map_or_else(String::new, human_size)).size(11.).color(rgb(t.secondary)));
                    }
                }
            });
            cancel = crate::icons::button(ui, crate::icons::Icon::X, false, i18n::t("Cancel")).clicked();
        });
    });
    cancel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(index: u8, album: Option<&str>, who: &str) -> DemoEvent {
        let scope = matrix_domain::RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!r:fixture.invalid") };
        DemoEvent {
            server_confirmed: true, redacted: false, annotation: String::new(), reactions: Vec::new(), transaction_id: None,
            id: matrix_domain::ScopedTimelineId { scope, id: TimelineId::Server(matrix_domain::EventId::new(format!("$e{index}{who}{}", album.unwrap_or("-")))) },
            text: String::new(), outgoing: false, send_state: None, timestamp_ms: 0, styles: Vec::new(), mentions: Vec::new(),
            highlighted: false, thread: None, thread_root: None, reply_to: None, status: false, status_row: None, utd: false,
            video: None, audio: None, poll: None, attachment_name: None, attachment_size: None, sender: Some(matrix_engine::history::HistorySender::new(who, None)),
            time: String::new(),
            media: Some(matrix_engine::history::Media {
                kind: MediaKind::Image,
                label: "Image".into(),
                animated: false,
                dims: Some((400, 300)),
                album: album.map(|id| matrix_engine::sending::Album { id: id.into(), index, count: 3 }),
            }),
        }
    }

    #[test]
    fn a_run_of_album_rows_is_drawn_once_on_the_last_and_a_lone_one_stays_single() {
        let rows = [ev(0, Some("a"), "@x:y"), ev(1, Some("a"), "@x:y"), ev(2, Some("a"), "@x:y"), ev(0, Some("b"), "@x:y"), ev(0, None, "@x:y"), ev(0, Some("c"), "@x:y"), ev(1, Some("c"), "@z:y")];
        assert_eq!(album_roles(&rows), [Role::Hidden, Role::Hidden, Role::Leader(0), Role::Single, Role::Single, Role::Single, Role::Single]);
        // Files of an album are cards of their own.
        let file = |i| {
            let mut e = ev(i, Some("f"), "@x:y");
            e.media.as_mut().unwrap().kind = MediaKind::File;
            e
        };
        let (file, file2) = (file(0), file(1));
        assert_eq!(album_roles(&[file, file2]), [Role::Single; 2]);
        // The same album id from another sender is another album; a row in between splits the run.
        assert_eq!(album_roles(&[ev(0, Some("a"), "@x:y"), ev(0, None, "@x:y"), ev(1, Some("a"), "@x:y")]), [Role::Single; 3]);
    }

    #[test]
    fn a_marked_album_is_bounded_by_its_count_and_never_hides_a_row_with_content() {
        let who = "@x:y";
        // 12 rows with the same id but a count of 3: runs of 3, the rest of their own.
        let rows: Vec<_> = (0..12).map(|_| ev(0, Some("a"), who)).collect();
        assert!(album_roles(&rows).iter().all(|r| *r == Role::Single), "equal indexes are not an album");
        let rows: Vec<_> = (0..12).map(|i| ev(i % 3, Some("a"), who)).collect();
        let roles = album_roles(&rows);
        assert_eq!(roles[..3], [Role::Hidden, Role::Hidden, Role::Leader(0)]);
        assert_eq!(roles.iter().filter(|r| **r == Role::Hidden).count(), 8);
        // A member in the middle with text ends the run: it is drawn, not hidden.
        let mut rows = [ev(0, Some("b"), who), ev(1, Some("b"), who), ev(2, Some("b"), who)];
        rows[1].text = "@tutti".into();
        assert_eq!(album_roles(&rows), [Role::Hidden, Role::Leader(0), Role::Single]);
    }

    #[test]
    fn the_grid_fills_the_width_in_every_row_and_never_overlaps() {
        for n in 1..=10 {
            let aspects: Vec<f32> = (0..n).map(|i| [1.33, 0.75, 1., 1.78, 0.56][i % 5]).collect();
            let (cells, height) = grid(&aspects, 300., 2.);
            assert_eq!(cells.len(), n);
            for c in &cells {
                assert!(c.min.x >= -0.01 && c.max.x <= 300.01 && c.min.y >= -0.01 && c.max.y <= height + 0.01, "{n}: {c:?}");
                assert!(c.width() > 20. && c.height() >= 60., "{n}: {c:?}");
            }
            for (i, a) in cells.iter().enumerate() {
                for b in &cells[i + 1..] {
                    assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{n}: {a:?} {b:?}");
                }
            }
            // A row ends at the right edge.
            assert!(cells.iter().any(|c| (c.max.x - 300.).abs() < 0.01));
        }
    }

    #[test]
    fn three_pictures_make_a_big_one_over_two() {
        let (cells, _) = grid(&[1.33, 1.33, 1.33], 300., 2.);
        assert!((cells[0].width() - 300.).abs() < 0.01);
        assert!(cells[1].min.y > cells[0].max.y && (cells[1].min.y - cells[2].min.y).abs() < 0.01 && cells[2].min.x > cells[1].max.x);
    }

    #[test]
    fn cover_crops_the_long_side_around_the_centre() {
        let wide = cover(vec2(400., 100.), vec2(100., 100.));
        assert!((wide.width() - 0.25).abs() < 1e-5 && (wide.center().x - 0.5).abs() < 1e-5 && wide.height() == 1.);
        let tall = cover(vec2(100., 400.), vec2(100., 100.));
        assert!((tall.height() - 0.25).abs() < 1e-5 && tall.width() == 1.);
        assert_eq!(cover(vec2(0., 0.), vec2(1., 1.)), Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)));
    }

    #[test]
    fn a_single_picture_keeps_its_shape_before_and_after_it_loads() {
        // Declared 3000x4000 (portrait): the tile has the shape the 180x240 thumbnail will have.
        let before = single_size(Some((3000, 4000)), None, 320., 320.);
        let after = single_size(Some((3000, 4000)), Some(vec2(180., 240.)), 320., 320.);
        assert!((before.x / before.y - after.x / after.y).abs() < 1e-3, "{before:?} {after:?}");
        assert_eq!(single_size(None, Some(vec2(320., 240.)), 200., 320.), vec2(200., 150.));
        assert_eq!(single_size(None, Some(vec2(100., 50.)), 320., 320.), vec2(100., 50.));
    }

    #[test]
    fn an_upload_with_no_byte_yet_is_indeterminate_then_it_fills() {
        assert_eq!(upload_fraction(0, Some(30 << 20)), None, "«0 B» must not be shown as a bar at rest");
        assert_eq!(upload_fraction(0, None), None);
        assert_eq!(upload_fraction(3 << 20, Some(30 << 20)), Some(0.1));
        assert_eq!(upload_fraction(5, None), None, "no total: nothing to divide by");
        assert_eq!(upload_fraction(30 << 20, Some(30 << 20)), Some(1.));
    }

    #[test]
    fn the_indeterminate_segment_moves_and_stays_inside_the_bar() {
        let spans: Vec<_> = (0..200).map(|i| indeterminate_span(i as f64 * 0.05)).collect();
        assert!(spans.iter().all(|(a, b)| *a >= 0. && *b <= 1.0001 && (b - a - 0.3).abs() < 1e-5), "{spans:?}");
        assert!(spans.windows(2).any(|w| w[0].0 != w[1].0), "it must move");
        let (first, mid) = (indeterminate_span(0.), indeterminate_span(0.8));
        assert!(first.0.abs() < 1e-6 && (mid.1 - 1.).abs() < 1e-5, "it runs from one end to the other: {first:?} {mid:?}");
    }

    #[test]
    fn the_upload_card_keeps_repainting_while_it_prepares() {
        let tokens = matrix_desktop::Theme::Dark.tokens();
        let frame = |sent: u64| {
            let ctx = egui::Context::default();
            let mut out = None;
            for _ in 0..2 {
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(400., 200.))), ..Default::default() };
                let mut frame = ctx.run_ui(raw, |ui| { upload_card(ui, "grande.bin", sent, 30 << 20, &tokens); });
                frame.textures_delta.clear();
                out = Some(frame);
            }
            out.unwrap().viewport_output.values().next().map(|v| v.repaint_delay).unwrap()
        };
        assert!(frame(0) <= std::time::Duration::from_millis(33), "the card keeps repainting while it prepares");
        frame(1 << 20); // the determinate bar draws too
    }

    #[test]
    fn file_families_follow_the_extension() {
        assert_eq!(family("Relazione.PDF"), (Family::Document, "PDF".into()));
        assert_eq!(family("a.tar.gz"), (Family::Archive, "GZ".into()));
        assert_eq!(family("senzaestensione"), (Family::Other, String::new()));
        assert_eq!(progress_text(1_048_576, Some(5_242_880)), "1.0 MB / 5.0 MB");
        assert_eq!(fraction(1, Some(4)), Some(0.25));
        assert_eq!(fraction(1, None), None);
        assert_eq!(fraction(9, Some(4)), Some(1.));
    }
}
