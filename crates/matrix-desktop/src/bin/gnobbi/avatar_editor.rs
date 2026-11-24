//! Retouching the profile picture before it is saved: move, zoom, rotate, mirror and colour.
//! The maths comes first and is toolkit-free; the window that drives it follows.
use crate::icons::{self, Icon};
use crate::list::rgb;
use eframe::egui::{self, RichText, Vec2};
use image::RgbaImage;
use matrix_desktop::{DesignTokens, i18n};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

/// Side of the square picture «Applica» produces.
pub const OUT_SIDE: u32 = 512;
/// Shorter side of the copy the preview is drawn from.
pub const PREVIEW_SIDE: u32 = 512;
/// Shorter side of the copy «Applica» reads: at 4× zoom the crop is still OUT_SIDE pixels, so nothing is lost.
pub const WORK_SIDE: u32 = 2048;
pub const MAX_ZOOM: f32 = 4.;
pub const MAX_FINE: f32 = 45.;

/// Where the square crop sits on the picture. The crop never leaves the picture, so the result has no empty corner.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pose {
    /// 1 = the largest crop the rotation allows.
    pub zoom: f32,
    /// Quarter turns clockwise.
    pub quarters: u8,
    /// Degrees clockwise on top of the quarter turns, within ±MAX_FINE.
    pub fine: f32,
    pub flip: bool,
    /// Centre of the crop, as a fraction of the picture's width and height.
    pub center: [f32; 2],
}

impl Default for Pose {
    fn default() -> Self {
        Self { zoom: 1., quarters: 0, fine: 0., flip: false, center: [0.5, 0.5] }
    }
}

impl Pose {
    fn sin_cos(&self) -> (f32, f32) {
        (f32::from(self.quarters % 4) * 90. + self.fine).to_radians().sin_cos()
    }

    /// Side of the crop in pixels of a `w`×`h` picture: a tilted square needs a smaller side to stay inside.
    pub fn side(&self, w: f32, h: f32) -> f32 {
        let (s, c) = self.sin_cos();
        w.min(h) / (s.abs() + c.abs()) / self.zoom
    }

    /// Brings zoom, angle and centre back into the range where the crop is fully on the picture.
    pub fn clamp(&mut self, w: f32, h: f32) {
        self.zoom = self.zoom.clamp(1., MAX_ZOOM);
        self.fine = self.fine.clamp(-MAX_FINE, MAX_FINE);
        // Half the bounding box of the tilted crop: side·(|sin|+|cos|)/2, which no longer depends on the angle.
        let reach = w.min(h) / (2. * self.zoom);
        for (c, len) in self.center.iter_mut().zip([w, h]) {
            *c = c.clamp(reach / len, 1. - reach / len);
        }
    }

    /// The picture pixel shown at (`u`, `v`) of the result, both in −0.5..0.5 from its centre.
    pub fn source(&self, u: f32, v: f32, w: f32, h: f32) -> [f32; 2] {
        let (s, c) = self.sin_cos();
        let side = self.side(w, h);
        let (x, y) = (if self.flip { -u } else { u } * side, v * side);
        [self.center[0] * w + c * x + s * y, self.center[1] * h - s * x + c * y]
    }

    /// The picture follows a drag of (`dx`, `dy`) points on a preview `view` points wide.
    pub fn drag(&mut self, dx: f32, dy: f32, view: f32, w: f32, h: f32) {
        let [x0, y0] = self.source(0., 0., w, h);
        let [x, y] = self.source(-dx / view, -dy / view, w, h);
        self.center[0] += (x - x0) / w;
        self.center[1] += (y - y0) / h;
        self.clamp(w, h);
    }

    /// One frame of a gesture: `zoom` multiplies, `rotation` (radians, clockwise on screen) adds to the tilt,
    /// `pan` (points) drags like one finger on a preview `view` points wide. Everything ends up clamped.
    pub fn apply_gesture(&mut self, zoom: f32, rotation: f32, pan: [f32; 2], view: f32, w: f32, h: f32) {
        self.zoom = (self.zoom * zoom).clamp(1., MAX_ZOOM);
        self.set_tilt((self.tilt() + rotation.to_degrees()).clamp(-MAX_FINE, MAX_FINE));
        self.drag(pan[0], pan[1], view, w, h);
    }

    /// A quarter turn as seen on screen (a mirrored picture turns the other way underneath).
    pub fn turn(&mut self, clockwise: bool) {
        self.quarters = (self.quarters + if clockwise != self.flip { 1 } else { 3 }) % 4;
    }

    /// The fine angle as seen on screen, clockwise.
    pub fn tilt(&self) -> f32 {
        if self.flip { -self.fine } else { self.fine }
    }

    pub fn set_tilt(&mut self, degrees: f32) {
        self.fine = if self.flip { -degrees } else { degrees };
    }

    /// Mirrors what is on screen left to right, whatever the rotation.
    pub fn mirror(&mut self) {
        self.flip = !self.flip;
    }

    /// Texture coordinates of the crop's corners (top-left, top-right, bottom-right, bottom-left).
    pub fn uv(&self, w: f32, h: f32) -> [[f32; 2]; 4] {
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(u, v)| {
            let [x, y] = self.source(u, v, w, h);
            [x / w, y / h]
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Filter {
    #[default]
    Original,
    Mono,
    Sepia,
    Vivid,
    Cool,
    Warm,
    Fade,
    Contrast,
}

impl Filter {
    pub const ALL: [Filter; 8] = [Filter::Original, Filter::Mono, Filter::Sepia, Filter::Vivid, Filter::Cool, Filter::Warm, Filter::Fade, Filter::Contrast];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Original => i18n::t("Original"),
            Filter::Mono => i18n::t("B&W"),
            Filter::Sepia => i18n::t("Sepia"),
            Filter::Vivid => i18n::t("Vivid"),
            Filter::Cool => i18n::t("Cool"),
            Filter::Warm => i18n::t("Warm"),
            Filter::Fade => i18n::t("Fade"),
            Filter::Contrast => i18n::t("High contrast"),
        }
    }

    /// Saturation, contrast, lift of the blacks, gain per channel.
    fn recipe(self) -> (f32, f32, f32, [f32; 3]) {
        match self {
            Filter::Original | Filter::Sepia => (1., 1., 0., [1.; 3]),
            Filter::Mono => (0., 1., 0., [1.; 3]),
            Filter::Vivid => (1.35, 1.1, 0., [1.; 3]),
            Filter::Cool => (1., 1., 0., [0.92, 1., 1.1]),
            Filter::Warm => (1., 1., 0., [1.1, 1., 0.9]),
            Filter::Fade => (0.8, 0.8, 0.06, [1.; 3]),
            Filter::Contrast => (1., 1.45, 0., [1.; 3]),
        }
    }
}

/// The colour of the result: one filter, then the three sliders (each −1..1, 0 = untouched).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Look {
    pub filter: Filter,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl Look {
    pub fn apply(&self, rgb: [u8; 3]) -> [u8; 3] {
        if *self == Look::default() {
            return rgb;
        }
        let (sat, con, lift, gain) = self.filter.recipe();
        let (sat, con, add) = (sat * (1. + self.saturation), con * (1. + self.contrast), lift + self.brightness * 0.4);
        let [r, g, b] = [0, 1, 2].map(|i| f32::from(rgb[i]) / 255. * gain[i]);
        let [r, g, b] = if self.filter == Filter::Sepia {
            // The classic matrix, a little darker so skin does not burn out.
            [0.393 * r + 0.769 * g + 0.189 * b, 0.349 * r + 0.686 * g + 0.168 * b, 0.272 * r + 0.534 * g + 0.131 * b].map(|c| c * 0.9)
        } else {
            [r, g, b]
        };
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        [r, g, b].map(|c| ((((luma + (c - luma) * sat) - 0.5) * con + 0.5 + add).clamp(0., 1.) * 255.).round() as u8)
    }

    /// Recolours a whole picture in place; transparency is kept.
    pub fn paint(&self, picture: &mut RgbaImage) {
        if *self != Look::default() {
            for p in picture.pixels_mut() {
                let [r, g, b] = self.apply([p[0], p[1], p[2]]);
                p.0 = [r, g, b, p[3]];
            }
        }
    }
}

/// A copy whose shorter side is at most `side` (the picture itself when already small).
pub fn bounded(picture: &RgbaImage, side: u32) -> RgbaImage {
    let (w, h) = picture.dimensions();
    let short = w.min(h);
    if short <= side {
        return picture.clone();
    }
    let k = f64::from(side) / f64::from(short);
    let scale = |v: u32| ((f64::from(v) * k).round() as u32).max(1);
    image::imageops::resize(picture, scale(w), scale(h), image::imageops::FilterType::Triangle)
}

/// The `out`×`out` result: the crop of `pose` read from `picture`, then the colours of `look`.
pub fn render(picture: &RgbaImage, pose: &Pose, look: &Look, out: u32) -> RgbaImage {
    let (w, h) = (picture.width() as f32, picture.height() as f32);
    let side = pose.side(w, h);
    // Shrinking by reading single pixels would alias: shrink the whole picture smoothly first, then read 1:1.
    if side > out as f32 * 1.01 {
        let short = (w.min(h) * out as f32 / side).round().max(1.) as u32;
        return render(&bounded(picture, short), pose, look, out);
    }
    let (max_x, max_y) = (picture.width() - 1, picture.height() - 1);
    let mut result = RgbaImage::from_fn(out, out, |x, y| {
        let at = |i: u32| (i as f32 + 0.5) / out as f32 - 0.5;
        let [sx, sy] = pose.source(at(x), at(y), w, h);
        // Pixel centres sit at .5: two neighbours on each axis, mixed by distance.
        let (fx, fy) = ((sx - 0.5).clamp(0., max_x as f32), (sy - 0.5).clamp(0., max_y as f32));
        let (x0, y0) = (fx as u32, fy as u32);
        let (x1, y1) = ((x0 + 1).min(max_x), (y0 + 1).min(max_y));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let mut px = [0u8; 4];
        for (i, v) in px.iter_mut().enumerate() {
            let p = |x, y| f32::from(picture.get_pixel(x, y)[i]);
            let (top, bottom) = (p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx, p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx);
            *v = (top + (bottom - top) * ty).round() as u8;
        }
        image::Rgba(px)
    });
    look.paint(&mut result);
    result
}

/// Side of the pictures in the filter row.
const THUMB_SIDE: u32 = 112;
/// Width of one column of controls; the preview is centred in it.
const COLUMN: f32 = 328.;
const PREVIEW: f32 = 240.;

/// A picture ready to retouch: the copy «Applica» reads, the one on screen and the one of the filter row.
struct Loaded {
    work: Arc<RgbaImage>,
    preview: RgbaImage,
    thumb: RgbaImage,
}

fn pixels(picture: &RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([picture.width() as usize, picture.height() as usize], picture.as_raw())
}

/// The retouch window of one picture. It outlives «Applica» (closed), so the same picture can be adjusted again.
#[derive(Default)]
pub struct Editor {
    pub open: bool,
    loading: Option<mpsc::Receiver<Result<Loaded, &'static str>>>,
    picture: Option<Loaded>,
    /// The preview with the colours of that look.
    texture: Option<(Look, egui::TextureHandle)>,
    thumbs: Vec<egui::TextureHandle>,
    pose: Pose,
    look: Look,
    /// What the last «Applica» used: «Annulla» goes back to it.
    applied: Option<(Pose, Look)>,
    saving: Option<mpsc::Receiver<Option<Vec<u8>>>>,
    /// Where «Applica» writes the result.
    out: PathBuf,
    error: Option<&'static str>,
    /// Test hook: «Applica» presses itself once the picture is there.
    pub auto_apply: bool,
}

impl Editor {
    fn start(out: PathBuf, ctx: &egui::Context, picture: impl FnOnce() -> Result<RgbaImage, &'static str> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let loaded = picture().map(|p| {
                let work = bounded(&p, WORK_SIDE);
                let preview = bounded(&work, PREVIEW_SIDE);
                let thumb = render(&preview, &Pose::default(), &Look::default(), THUMB_SIDE);
                Loaded { work: Arc::new(work), preview, thumb }
            });
            let _ = tx.send(loaded);
            ctx.request_repaint();
        });
        Self { open: true, loading: Some(rx), out, ..Default::default() }
    }

    /// A picture chosen by the user: same formats and limits as the upload.
    pub fn from_file(path: PathBuf, out: PathBuf, ctx: &egui::Context) -> Self {
        Self::start(out, ctx, move || matrix_engine::profile::decode_blocking(&path).map(|p| p.into_rgba8()).map_err(|e| e.message()))
    }

    /// The picture the account has now, as the server holds it (`None`: the account is not connected).
    pub fn from_server(original: Option<mpsc::Receiver<matrix_engine::avatar::AvatarPicture>>, out: PathBuf, ctx: &egui::Context) -> Self {
        Self::start(out, ctx, move || {
            let (w, h, rgba) = original.and_then(|rx| rx.recv().ok()).and_then(|p| p.rgba).ok_or("Could not download the current photo.")?;
            RgbaImage::from_raw(w, h, rgba).ok_or("Could not download the current photo.")
        })
    }

    /// Closed without ever pressing «Applica»: nothing of it is in use.
    pub fn unused(&self) -> bool {
        !self.open && self.applied.is_none()
    }

    /// Test hook, debug builds only: GNOBBI_EGUI_AVATAR_EDIT="<filter 0-7> <zoom> <degrees> <brightness>".
    #[cfg(debug_assertions)]
    fn hook(&mut self) {
        if let Ok(v) = matrix_desktop::hooks::var("GNOBBI_EGUI_AVATAR_EDIT") {
            let mut n = v.split_whitespace().map(|p| p.parse::<f32>().unwrap_or(0.));
            self.look.filter = Filter::ALL[(n.next().unwrap_or(0.) as usize).min(7)];
            self.pose.zoom = n.next().unwrap_or(1.);
            self.pose.fine = n.next().unwrap_or(0.);
            self.look.brightness = n.next().unwrap_or(0.);
        }
    }

    fn apply(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.picture else { return };
        let (tx, rx) = mpsc::channel();
        let (work, pose, look, out, ctx) = (p.work.clone(), self.pose, self.look, self.out.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = render(&work, &pose, &look, OUT_SIDE);
            let small = image::imageops::resize(&result, 128, 128, image::imageops::FilterType::Triangle);
            let _ = tx.send(result.save_with_format(&out, image::ImageFormat::Png).ok().map(|()| small.into_raw()));
            ctx.request_repaint();
        });
        self.saving = Some(rx);
        self.error = None;
    }

    fn close(&mut self) {
        self.open = false;
        if let Some((pose, look)) = self.applied {
            (self.pose, self.look) = (pose, look);
        }
    }

    /// Draws the window while open. «Applica» closes it and hands back the file written and its small preview.
    pub fn show(&mut self, ctx: &egui::Context, t: &DesignTokens) -> Option<(PathBuf, egui::TextureHandle)> {
        if !self.open {
            return None;
        }
        if let Some(Ok(done)) = self.loading.as_ref().map(|rx| rx.try_recv()) {
            self.loading = None;
            match done {
                Ok(p) => {
                    self.thumbs = Filter::ALL
                        .iter()
                        .map(|&filter| {
                            let mut small = p.thumb.clone();
                            Look { filter, ..Default::default() }.paint(&mut small);
                            ctx.load_texture(format!("avatar-filter-{filter:?}"), pixels(&small), Default::default())
                        })
                        .collect();
                    self.picture = Some(p);
                    #[cfg(debug_assertions)]
                    self.hook();
                    if self.auto_apply {
                        self.apply(ctx);
                    }
                }
                Err(line) => self.error = Some(line),
            }
        }
        if let Some(Ok(done)) = self.saving.as_ref().map(|rx| rx.try_recv()) {
            self.saving = None;
            match done {
                Some(small) => {
                    self.applied = Some((self.pose, self.look));
                    self.open = false;
                    let small = egui::ColorImage::from_rgba_unmultiplied([128, 128], &small);
                    return Some((self.out.clone(), ctx.load_texture("own-avatar-preview", small, Default::default())));
                }
                None => self.error = Some("Could not prepare the photo."),
            }
        }
        if let Some(p) = &self.picture {
            let (w, h) = (p.preview.width() as f32, p.preview.height() as f32);
            self.pose.clamp(w, h);
            if self.texture.as_ref().is_none_or(|(look, _)| *look != self.look) {
                let mut shown = p.preview.clone();
                self.look.paint(&mut shown);
                self.texture = Some((self.look, ctx.load_texture("avatar-editor", pixels(&shown), Default::default())));
            }
        }
        let screen = ctx.content_rect().size();
        let wide = screen.x >= 2. * COLUMN + 120.;
        let touch = crate::mobile::touch();
        // On touch the column takes the whole width.
        let column = if touch && !wide { screen.x - 72. } else { COLUMN.min(screen.x - 72.) };
        let (mut close, mut apply, mut reset) = (false, false, false);
        let frame = egui::Frame::popup(&ctx.style_of(ctx.theme())).fill(rgb(t.surface)).stroke(egui::Stroke::NONE).corner_radius(12).inner_margin(20);
        let modal = egui::Modal::new(egui::Id::new("avatar-editor")).frame(frame).show(ctx, |ui| crate::dir::scope(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.;
            let width = if wide { 2. * column + 24. } else { column };
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(RichText::new(i18n::t("Profile photo")).size(crate::metrics::get().title).strong().color(rgb(t.text)));
                ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| close |= icons::button(ui, Icon::X, false, i18n::t("Close")).clicked());
            });
            let ready = self.picture.is_some();
            // Title and buttons stay in view; a short window scrolls what is between them.
            let room = (screen.y - 72. - 2. * crate::metrics::button() - 32.).max(120.);
            egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
                ui.add_enabled_ui(ready && self.saving.is_none(), |ui| {
                    if wide {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 24.;
                            ui.with_layout(crate::dir::col(), |ui| self.frame_side(ui, column, t));
                            ui.with_layout(crate::dir::col(), |ui| self.colour_side(ui, column, t));
                        });
                    } else {
                        self.frame_side(ui, column, t);
                        self.colour_side(ui, column, t);
                    }
                });
            });
            if let Some(line) = self.error {
                ui.add(egui::Label::new(RichText::new(i18n::t(line)).color(ui.visuals().error_fg_color)).truncate());
            }
            ui.horizontal(|ui| {
                reset = ui.add_enabled(ready, egui::Button::new(icons::with_label(Icon::ArrowUUpLeft, i18n::t("Reset")))).clicked();
                ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
                    if self.saving.is_some() {
                        ui.spinner();
                    } else {
                        apply = ui.add_enabled(ready, egui::Button::new(RichText::new(i18n::t("Apply")).color(rgb(t.on_accent))).fill(rgb(t.accent))).clicked();
                    }
                    close |= ui.button(i18n::t("Cancel")).clicked();
                });
            });
        }));
        if reset {
            (self.pose, self.look) = Default::default();
        }
        if apply {
            self.apply(ctx);
        } else if (close || modal.should_close()) && self.saving.is_none() {
            self.close();
        }
        None
    }

    /// The round preview with move, zoom and rotation.
    fn frame_side(&mut self, ui: &mut egui::Ui, column: f32, t: &DesignTokens) {
        // On touch the preview is a full-width square, but not taller than a third of the screen, so the filters stay in view.
        let side = if crate::mobile::touch() { column.min((ui.ctx().content_rect().height() * 0.36).max(160.)) } else { PREVIEW.min(column) };
        let (row, _) = ui.allocate_exact_size(Vec2::new(column, side), egui::Sense::hover());
        let rect = egui::Rect::from_center_size(row.center(), Vec2::splat(side));
        let (c, r) = (rect.center(), side / 2.);
        let shown = self.picture.as_ref().zip(self.texture.as_ref()).map(|(p, (_, texture))| (p.preview.width() as f32, p.preview.height() as f32, texture.id()));
        if let Some((w, h, texture)) = shown {
            let resp = ui.interact(rect, ui.id().with("avatar-preview"), egui::Sense::click_and_drag());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, i18n::t("Preview: drag to move the photo")));
            if resp.clicked() {
                resp.request_focus();
            }
            // Two fingers that started on the preview: egui's own pinch, twist and pan (their zoom is also in `zoom_delta`).
            let touch = ui.input(|i| i.multi_touch()).filter(|m| rect.contains(m.start_pos));
            let pan = touch.map_or_else(|| resp.drag_delta(), |m| m.translation_delta);
            let (turn, mut factor) = (touch.map_or(0., |m| m.rotation_delta), 1.);
            if touch.is_some() || resp.hovered() {
                // Wheel and trackpad pinch zoom here instead of scrolling the window.
                factor = ui.input(|i| i.zoom_delta() * (i.smooth_scroll_delta.y / 600.).exp());
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
            if resp.dragged() || touch.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            self.pose.apply_gesture(factor, turn, [pan.x, pan.y], side, w, h);
            if resp.has_focus() {
                // Arrows move the picture instead of the focus; plus and minus zoom.
                ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
                let key = |k| ui.input(|i| i.key_pressed(k));
                let step = |plus, minus| if key(plus) { 12. } else if key(minus) { -12. } else { 0. };
                self.pose.drag(step(egui::Key::ArrowRight, egui::Key::ArrowLeft), step(egui::Key::ArrowDown, egui::Key::ArrowUp), side, w, h);
                self.pose.zoom *= 1. + step(egui::Key::Plus, egui::Key::Minus) / 100.;
            }
            self.pose.clamp(w, h);
            let mut mesh = egui::Mesh::with_texture(texture);
            for (pos, [u, v]) in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].into_iter().zip(self.pose.uv(w, h)) {
                mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::pos2(u, v), color: egui::Color32::WHITE });
            }
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            ui.painter().add(egui::Shape::mesh(mesh));
            // What the round avatar leaves out is veiled: a ring from the circle to the square, in the window's own colour.
            let mut veil = egui::Mesh::default();
            let fill = rgb(t.surface).gamma_multiply(0.7);
            const STEPS: u32 = 64;
            for i in 0..=STEPS {
                let (sin, cos) = (i as f32 / STEPS as f32 * std::f32::consts::TAU).sin_cos();
                let d = Vec2::new(cos, sin);
                veil.colored_vertex(c + d * r, fill);
                veil.colored_vertex(c + d * r / cos.abs().max(sin.abs()), fill);
            }
            for k in (0..STEPS).map(|i| i * 2) {
                veil.add_triangle(k, k + 1, k + 3);
                veil.add_triangle(k, k + 3, k + 2);
            }
            ui.painter().add(egui::Shape::mesh(veil));
            // Rounds the square's corners like the filter pictures: a band in the window's colour just outside a rounded outline.
            ui.painter().rect_stroke(rect, 12., egui::Stroke::new(6., rgb(t.surface)), egui::StrokeKind::Outside);
            ui.painter().circle_stroke(c, r - 1., egui::Stroke::new(2., if resp.has_focus() { rgb(t.accent) } else { rgb(t.surface) }));
        } else {
            ui.painter().circle_filled(c, r, rgb(t.input));
            if self.error.is_none() {
                egui::Spinner::new().paint_at(ui, egui::Rect::from_center_size(c, Vec2::splat(28.)));
            }
        }
        let m = crate::metrics::get();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = m.gap;
            ui.add_space(((column - 3. * crate::metrics::button() - 2. * m.gap) / 2.).max(0.));
            if icons::button(ui, Icon::ArrowCounterClockwise, false, i18n::t("Rotate left")).clicked() {
                self.pose.turn(false);
            }
            if icons::button(ui, Icon::ArrowClockwise, false, i18n::t("Rotate right")).clicked() {
                self.pose.turn(true);
            }
            if icons::button(ui, Icon::FlipHorizontal, self.pose.flip, i18n::t("Mirror")).clicked() {
                self.pose.mirror();
            }
        });
        // Fingers zoom and turn the preview; no sliders on touch.
        if !show_sliders(crate::mobile::touch()) {
            return;
        }
        let names = [i18n::t("Zoom"), i18n::t("Rotation")];
        let wide = label_width(ui, &names, column);
        slider(ui, names[0], wide, &mut self.pose.zoom, 1. ..=MAX_ZOOM, column, t, |v| format!("{v:.1}×"));
        let mut tilt = self.pose.tilt();
        slider(ui, names[1], wide, &mut tilt, -MAX_FINE..=MAX_FINE, column, t, |v| format!("{v:+.0}°"));
        self.pose.set_tilt(tilt);
    }

    /// The filter row and the three colour sliders.
    fn colour_side(&mut self, ui: &mut egui::Ui, column: f32, t: &DesignTokens) {
        const GAP: f32 = 8.;
        let touch = crate::mobile::touch();
        let cell = if touch { Vec2::new(64., 84.) } else { Vec2::new((column - 3. * GAP) / 4., 84.) };
        let per_line = if touch { Filter::ALL.len() } else { 4 };
        let mut lines = |ui: &mut egui::Ui| {
            for line in Filter::ALL.chunks(per_line) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for &filter in line {
                        let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
                        let resp = crate::metrics::clickable(resp);
                        let on = self.look.filter == filter;
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, filter.label()));
                        let picture = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 32.), Vec2::splat(56.));
                        match self.thumbs.get(Filter::ALL.iter().position(|f| *f == filter).unwrap_or(0)) {
                            Some(texture) => egui::Image::new(texture).corner_radius(egui::CornerRadius::same(10)).paint_at(ui, picture),
                            None => {
                                ui.painter().rect_filled(picture, 10., rgb(t.input));
                            }
                        }
                        if on || resp.hovered() || resp.has_focus() {
                            let ring = if on { rgb(t.accent) } else { rgb(t.secondary) };
                            ui.painter().rect_stroke(picture.expand(3.), 13., egui::Stroke::new(2., ring), egui::StrokeKind::Inside);
                        }
                        let ink = if on { rgb(t.text) } else { rgb(t.secondary) };
                        let name = crate::list::one_line(ui, filter.label(), 11., ink, cell.x);
                        ui.painter().galley(egui::pos2(rect.center().x - name.size().x / 2., picture.bottom() + 7.), name, ink);
                        if resp.clicked() {
                            self.look.filter = filter;
                        }
                    }
                });
            }
        };
        if touch {
            // One row that scrolls sideways; a vertical drag on it is passed on to the window's scroll.
            let row = egui::ScrollArea::horizontal().id_salt("avatar-filters").show(ui, |ui| lines(ui));
            if let Some(drag) = ui.ctx().read_response(row.id.with("area")).filter(|r| r.dragged()) {
                ui.scroll_with_delta(Vec2::new(0., drag.drag_delta().y));
            }
        } else {
            lines(ui);
        }
        let percent = |v: f32| format!("{:+.0}", v * 100.);
        let names = [i18n::t("Brightness"), i18n::t("Contrast"), i18n::t("Saturation")];
        let wide = label_width(ui, &names, column);
        slider(ui, names[0], wide, &mut self.look.brightness, -1. ..=1., column, t, percent);
        slider(ui, names[1], wide, &mut self.look.contrast, -1. ..=1., column, t, percent);
        slider(ui, names[2], wide, &mut self.look.saturation, -1. ..=1., column, t, percent);
    }
}

/// The Zoom and Rotation sliders are for mouse users: on touch the fingers do it.
fn show_sliders(touch: bool) -> bool {
    !touch
}

/// Width of the name column of a group of sliders: the longest name in the current language, at most 40% of the column (then it is cut with «…»).
fn label_width(ui: &egui::Ui, names: &[&str], column: f32) -> f32 {
    let size = crate::metrics::get().body;
    names.iter().map(|n| crate::list::one_line(ui, n, size, egui::Color32::WHITE, f32::INFINITY).size().x).fold(0., f32::max).min(column * 0.4).ceil() + 8.
}

/// One slider row: name on the left, value on the right; the screen reader hears both.
fn slider(ui: &mut egui::Ui, label: &str, label_w: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>, column: f32, t: &DesignTokens, shown: impl Fn(f32) -> String) {
    const VALUE: f32 = 44.;
    let m = crate::metrics::get();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = m.gap;
        let height = crate::metrics::button();
        let (name, _) = ui.allocate_exact_size(Vec2::new(label_w, height), egui::Sense::hover());
        let text = crate::list::one_line(ui, label, m.body, rgb(t.text), label_w);
        ui.painter().galley(crate::dir::place(name, egui::pos2(name.left(), name.center().y - text.size().y / 2.), text.size().x), text, rgb(t.text));
        ui.spacing_mut().slider_width = column - label_w - VALUE - 2. * m.gap;
        ui.visuals_mut().selection.bg_fill = rgb(t.accent);
        let resp = ui.add(egui::Slider::new(value, range).show_value(false).trailing_fill(true).handle_shape(egui::style::HandleShape::Circle));
        resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(*value), format!("{label} {}", shown(*value))));
        let (number, _) = ui.allocate_exact_size(Vec2::new(VALUE, height), egui::Sense::hover());
        ui.painter().text(crate::dir::flip_pos(number, number.right_center()), crate::dir::align(egui::Align2::RIGHT_CENTER), shown(*value), egui::FontId::proportional(m.secondary), rgb(t.secondary));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    /// Four flat quarters: red top-left, green top-right, blue bottom-right, white bottom-left.
    fn quarters(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| image::Rgba(match (x < w / 2, y < h / 2) {
            (true, true) => RED,
            (false, true) => GREEN,
            (false, false) => BLUE,
            (true, false) => WHITE,
        }))
    }

    /// The colours at the centre of the four quarters of a rendered result, clockwise from top-left.
    fn corners(out: &RgbaImage) -> [[u8; 4]; 4] {
        let (a, b) = (out.width() / 4, out.width() * 3 / 4);
        [(a, a), (b, a), (b, b), (a, b)].map(|(x, y)| out.get_pixel(x, y).0)
    }

    #[test]
    fn the_crop_stays_on_the_picture_at_any_angle_zoom_and_position() {
        for (w, h) in [(800., 600.), (600., 800.), (512., 512.), (3000., 200.)] {
            for quarters in 0..4 {
                for fine in [-80., -45., -30., -1., 0., 7.5, 45., 90.] {
                    for zoom in [0.2, 1., 1.7, 4., 9.] {
                        for center in [[0.5, 0.5], [0., 0.], [1., 1.], [-3., 0.4], [0.9, 7.]] {
                            let mut p = Pose { zoom, quarters, fine, flip: quarters % 2 == 0, center };
                            p.clamp(w, h);
                            assert!((1. ..=MAX_ZOOM).contains(&p.zoom) && p.fine.abs() <= MAX_FINE);
                            for [u, v] in p.uv(w, h) {
                                assert!((-1e-4..=1.0001).contains(&u) && (-1e-4..=1.0001).contains(&v), "{p:?} on {w}x{h} leaves the picture: {u},{v}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_untouched_pose_is_the_centre_square_and_a_tilt_shrinks_it() {
        let p = Pose::default();
        assert_eq!(p.side(800., 600.), 600.);
        assert_eq!(p.uv(800., 600.), [[0.125, 0.], [0.875, 0.], [0.875, 1.], [0.125, 1.]]);
        let tilted = Pose { fine: 45., ..p };
        assert!((tilted.side(600., 600.) - 600. / 2f32.sqrt()).abs() < 0.01);
        assert_eq!(Pose { zoom: 4., ..p }.side(800., 600.), 150.);
    }

    #[test]
    fn zoom_one_on_a_wide_picture_slides_only_sideways() {
        let mut p = Pose::default();
        p.drag(-1000., 500., 240., 800., 600.);
        assert_eq!(p.center, [0.625, 0.5], "pulled left as far as the right edge allows, no room up or down");
        p.drag(40., 0., 240., 800., 600.);
        assert!((p.center[0] - (0.625 - 100. / 800.)).abs() < 1e-5, "40 of 240 points is a sixth of the 600 px crop");
    }

    #[test]
    fn a_gesture_zooms_turns_and_pans_within_the_limits() {
        let mut p = Pose::default();
        p.apply_gesture(1.5, 0.1, [0., 0.], 240., 400., 400.);
        assert!((p.zoom - 1.5).abs() < 1e-6 && (p.tilt() - 0.1f32.to_degrees()).abs() < 1e-4);
        p.apply_gesture(100., 10., [0., 0.], 240., 400., 400.);
        assert_eq!((p.zoom, p.fine), (MAX_ZOOM, MAX_FINE));
        p.apply_gesture(0.001, -10., [0., 0.], 240., 400., 400.);
        assert_eq!((p.zoom, p.fine), (1., -MAX_FINE));
        let (mut a, mut b) = (Pose { zoom: 2., ..Default::default() }, Pose { zoom: 2., ..Default::default() });
        a.apply_gesture(1., 0., [24., -12.], 240., 400., 400.);
        b.drag(24., -12., 240., 400., 400.);
        assert_eq!(a, b, "one finger is the existing drag");
    }

    #[test]
    fn a_drag_follows_the_finger_on_a_turned_or_mirrored_picture() {
        let turned = |quarters, flip| {
            let mut p = Pose { zoom: 2., quarters, flip, ..Default::default() };
            p.drag(24., 0., 240., 400., 400.);
            [(p.center[0] * 400.).round(), (p.center[1] * 400.).round()]
        };
        assert_eq!(turned(0, false), [180., 200.], "dragging right shows what was on the left");
        assert_eq!(turned(0, true), [220., 200.]);
        assert_eq!(turned(1, false), [200., 220.], "after a quarter turn the screen's left is the picture's bottom");
    }

    #[test]
    fn render_turns_clockwise_and_mirrors() {
        let picture = quarters(64, 64);
        let shot = |pose: Pose| corners(&render(&picture, &pose, &Look::default(), 32));
        assert_eq!(shot(Pose::default()), [RED, GREEN, BLUE, WHITE]);
        assert_eq!(shot(Pose { quarters: 1, ..Default::default() }), [WHITE, RED, GREEN, BLUE]);
        assert_eq!(shot(Pose { quarters: 2, ..Default::default() }), [BLUE, WHITE, RED, GREEN]);
        assert_eq!(shot(Pose { flip: true, ..Default::default() }), [GREEN, RED, WHITE, BLUE]);
        assert_eq!(shot(Pose { quarters: 1, flip: true, ..Default::default() }), [RED, WHITE, BLUE, GREEN]);
    }

    #[test]
    fn buttons_act_on_what_is_on_screen_even_when_mirrored() {
        let picture = quarters(64, 64);
        let shot = |pose: &Pose| corners(&render(&picture, pose, &Look::default(), 32));
        let mut p = Pose::default();
        p.mirror();
        assert_eq!(shot(&p), [GREEN, RED, WHITE, BLUE]);
        p.turn(true);
        assert_eq!(shot(&p), [BLUE, GREEN, RED, WHITE], "the mirrored picture, a quarter turn clockwise");
        p.mirror();
        assert_eq!(shot(&p), [GREEN, BLUE, WHITE, RED], "left and right swap on screen");
        p.turn(false);
        p.turn(false);
        p.turn(true);
        p.turn(true);
        assert_eq!(shot(&p), [GREEN, BLUE, WHITE, RED]);
        let mut tilted = Pose { flip: true, ..Default::default() };
        tilted.set_tilt(20.);
        assert_eq!((tilted.fine, tilted.tilt()), (-20., 20.));
    }

    #[test]
    fn render_crops_the_square_the_pose_points_at() {
        // 200x100: at zoom 1 the centre square holds the inner halves of all four quarters.
        let picture = quarters(200, 100);
        let out = render(&picture, &Pose::default(), &Look::default(), OUT_SIDE);
        assert_eq!(out.dimensions(), (OUT_SIDE, OUT_SIDE));
        assert_eq!(corners(&out), [RED, GREEN, BLUE, WHITE]);
        // Zoom 2 on the top-left quarter: all red, enlarged from 50 px.
        let mut p = Pose { zoom: 2., center: [0., 0.], ..Default::default() };
        p.clamp(200., 100.);
        assert_eq!(p.center, [0.125, 0.25]);
        assert_eq!(corners(&render(&picture, &p, &Look::default(), 64)), [RED; 4]);
        // A tilt never shows an empty (transparent) corner.
        let tilted = render(&picture, &Pose { fine: 33., ..Default::default() }, &Look::default(), 64);
        assert!(tilted.pixels().all(|p| p[3] == 255));
        // A large picture is shrunk smoothly and keeps its layout.
        assert_eq!(corners(&render(&quarters(3000, 2000), &Pose::default(), &Look::default(), 64)), [RED, GREEN, BLUE, WHITE]);
    }

    #[test]
    fn bounded_keeps_small_pictures_and_shrinks_the_shorter_side() {
        assert_eq!(bounded(&quarters(40, 30), 512).dimensions(), (40, 30));
        assert_eq!(bounded(&quarters(4000, 1000), 500).dimensions(), (2000, 500));
    }

    #[test]
    fn filters_on_sample_pixels() {
        let with = |filter| Look { filter, ..Default::default() };
        let px = [200, 100, 50];
        assert_eq!(with(Filter::Original).apply(px), px);
        let [r, g, b] = with(Filter::Mono).apply(px);
        assert_eq!((r, g, b), (124, 124, 124), "luma 0.299 0.587 0.114");
        assert_eq!(with(Filter::Sepia).apply(px), [148, 132, 103]);
        assert_eq!(with(Filter::Sepia).apply([255, 255, 255]), [255, 255, 215], "bright values clip, never wrap");
        let [r, _, b] = with(Filter::Cool).apply(px);
        assert!(r < 200 && b > 50);
        let [r, _, b] = with(Filter::Warm).apply(px);
        assert!(r > 200 && b < 50);
        let vivid = with(Filter::Vivid).apply(px);
        assert!(vivid[0] > 200 && vivid[2] < 50, "more colour: {vivid:?}");
        assert_eq!(with(Filter::Fade).apply([0, 0, 0]), [41, 41, 41], "blacks are lifted");
        assert_eq!(with(Filter::Contrast).apply([128, 128, 128]), [128, 128, 128], "mid grey stays");
        assert!(with(Filter::Contrast).apply([64, 64, 64])[0] < 64 && with(Filter::Contrast).apply([192, 192, 192])[0] > 192);
        for f in Filter::ALL {
            assert!(!f.label().is_empty());
        }
    }

    #[test]
    fn sliders_on_sample_pixels() {
        let px = [200, 100, 50];
        assert!(Look { brightness: 0.5, ..Default::default() }.apply(px).iter().zip(px).all(|(a, b)| *a > b));
        assert_eq!(Look { brightness: 1., ..Default::default() }.apply([250, 250, 250]), [255, 255, 255]);
        assert_eq!(Look { brightness: -1., ..Default::default() }.apply([10, 10, 10]), [0, 0, 0]);
        let grey = Look { saturation: -1., ..Default::default() }.apply(px);
        assert!(grey[0] == grey[1] && grey[1] == grey[2]);
        assert_eq!(Look { contrast: -1., ..Default::default() }.apply(px), [128, 128, 128], "no contrast is flat grey");
        // Filter and sliders add up: black and white stays grey whatever the saturation slider says.
        let mono = Look { filter: Filter::Mono, saturation: 1., ..Default::default() }.apply(px);
        assert!(mono[0] == mono[1] && mono[1] == mono[2]);
    }

    #[test]
    fn paint_keeps_transparency() {
        let mut picture = RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 7]));
        Look { filter: Filter::Mono, ..Default::default() }.paint(&mut picture);
        assert_eq!(picture.get_pixel(1, 1).0, [124, 124, 124, 7]);
    }

    #[test]
    fn zoom_and_rotation_sliders_are_desktop_only() {
        assert!(show_sliders(false));
        assert!(!show_sliders(true));
    }

    #[test]
    fn neutral_gesture_deltas_change_nothing() {
        let mut p = Pose { zoom: 2.5, quarters: 1, fine: 12., flip: true, center: [0.4, 0.6] };
        let before = p;
        p.apply_gesture(1., 0., [0., 0.], 240., 400., 400.);
        assert_eq!(p, before);
    }

    #[test]
    fn zoom_delta_is_multiplicative_and_stops_at_one_and_the_maximum() {
        let zoomed = |start: f32, factor: f32| {
            let mut p = Pose { zoom: start, ..Default::default() };
            p.apply_gesture(factor, 0., [0., 0.], 240., 400., 400.);
            p.zoom
        };
        assert_eq!(zoomed(1., 0.5), 1., "pinching in below 1 stays at 1");
        assert_eq!(zoomed(2., 0.5), 1.);
        assert_eq!(zoomed(2., 1.5), 3.);
        assert_eq!(zoomed(3., 2.), MAX_ZOOM, "above the maximum stays at the maximum");
        assert_eq!(zoomed(MAX_ZOOM, 1.01), MAX_ZOOM);
        let mut p = Pose::default();
        for _ in 0..3 {
            p.apply_gesture(1.2, 0., [0., 0.], 240., 400., 400.);
        }
        assert!((p.zoom - 1.728).abs() < 1e-4, "frames accumulate: {}", p.zoom);
    }

    #[test]
    fn rotation_delta_accumulates_in_degrees_and_stops_at_the_fine_limit() {
        let mut p = Pose::default();
        for _ in 0..3 {
            p.apply_gesture(1., 10f32.to_radians(), [0., 0.], 240., 400., 400.);
        }
        assert!((p.tilt() - 30.).abs() < 1e-3, "{}", p.tilt());
        for _ in 0..10 {
            p.apply_gesture(1., 10f32.to_radians(), [0., 0.], 240., 400., 400.);
        }
        assert_eq!(p.tilt(), MAX_FINE);
        p.apply_gesture(1., -std::f32::consts::TAU, [0., 0.], 240., 400., 400.);
        assert_eq!(p.tilt(), -MAX_FINE);
        let mut mirrored = Pose { flip: true, ..Default::default() };
        mirrored.apply_gesture(1., 10f32.to_radians(), [0., 0.], 240., 400., 400.);
        assert!((mirrored.tilt() - 10.).abs() < 1e-3 && mirrored.fine < 0., "clockwise on screen, whatever the mirror");
        assert_eq!(mirrored.quarters, 0, "a gesture never changes the quarter turns");
    }

    #[test]
    fn translation_delta_stops_at_the_picture_edges() {
        let mut p = Pose { zoom: 2., ..Default::default() };
        // 400x400 at zoom 2: the crop is 200 px, so its centre stays within 0.25..0.75.
        p.apply_gesture(1., 0., [-100_000., -100_000.], 240., 400., 400.);
        assert_eq!(p.center, [0.75, 0.75], "dragged left and up: the picture shows its bottom right");
        p.apply_gesture(1., 0., [100_000., 100_000.], 240., 400., 400.);
        assert_eq!(p.center, [0.25, 0.25]);
        let mut at_one = Pose::default();
        at_one.apply_gesture(1., 0., [50., -50.], 240., 400., 400.);
        assert_eq!(at_one.center, [0.5, 0.5], "zoom 1 on a square picture: no room to move");
    }

    /// Fixed pose, look and picture: sample pixels of the result, taken from the code before the gestures were added.
    #[test]
    fn render_of_fixed_parameters_is_unchanged() {
        let picture = RgbaImage::from_fn(96, 64, |x, y| image::Rgba([(x * 2) as u8, (y * 4) as u8, ((x + y) * 3 % 256) as u8, 255]));
        let pose = Pose { zoom: 1.5, quarters: 1, fine: 12., flip: true, center: [0.5, 0.5] };
        let look = Look { filter: Filter::Sepia, brightness: 0.2, contrast: -0.1, saturation: 0.3 };
        let out = render(&picture, &pose, &look, 32);
        let golden = [[118, 107, 85], [199, 176, 135], [185, 164, 126], [135, 120, 95], [206, 182, 139], [160, 142, 111]];
        for ((x, y), want) in [(0, 0), (31, 0), (16, 16), (0, 31), (31, 31), (8, 24)].into_iter().zip(golden) {
            let got = out.get_pixel(x, y).0;
            // ±1: float rounding may differ between CPUs, a real change moves a pixel by far more.
            assert!(got[3] == 255 && got[..3].iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1), "({x},{y}): {got:?} instead of {want:?}");
        }
    }
}
