use eframe::egui;
use matrix_domain::EventId;
use matrix_engine::media::{MediaError, Preview};
use std::path::PathBuf;

pub enum Action { None, Close, Save(PathBuf), Open }

fn fit(image: egui::Vec2, available: egui::Vec2) -> egui::Vec2 {
    image * (available.x / image.x).min(available.y / image.y)
}

pub struct Viewer {
    pub id: EventId,
    video: bool,
    name: String,
    original: Option<egui::TextureHandle>,
    pub failed: bool,
    pub opening: bool,
    player: Option<std::sync::mpsc::Receiver<bool>>,
}

impl Viewer {
    pub fn new(id: EventId, video: bool, name: String) -> Self {
        Self { id, video, name, original: None, failed: false, opening: false, player: None }
    }

    pub fn set_original(&mut self, ctx: &egui::Context, result: Result<Preview, MediaError>) {
        match result {
            Ok(p) => {
                let image = egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba);
                self.original = Some(ctx.load_texture(format!("original-{}", self.id.as_str()), image, Default::default()));
            }
            Err(_) => self.failed = true,
        }
    }

    pub fn open_player(&mut self, path: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.player = Some(rx);
        std::thread::spawn(move || {
            let program = std::env::var("GNOBBI_VIDEO_OPENER").unwrap_or_else(|_| if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" }.into());
            let ok = std::process::Command::new(program).arg(path).status().is_ok_and(|s| s.success());
            let _ = tx.send(ok);
        });
    }

    pub fn show(&mut self, ctx: &egui::Context, thumbnail: Option<&egui::TextureHandle>) -> Action {
        if let Some(rx) = &self.player && let Ok(ok) = rx.try_recv() {
            self.player = None;
            self.opening = false;
            if ok { return Action::Close; }
            self.failed = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) { return Action::Close; }
        let mut action = Action::None;
        let size = ctx.content_rect().size();
        egui::Window::new(if self.video { "Video" } else { "Immagine" })
            .collapsible(false).resizable(true).default_size(size * 0.8)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Painted X: the font has no glyph for the cross character on every system.
                    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(24.), egui::Sense::click());
                    let c = rect.center();
                    let stroke = egui::Stroke::new(1.6, ui.visuals().text_color());
                    ui.painter().line_segment([c + egui::vec2(-5., -5.), c + egui::vec2(5., 5.)], stroke);
                    ui.painter().line_segment([c + egui::vec2(-5., 5.), c + egui::vec2(5., -5.)], stroke);
                    if resp.clicked() { action = Action::Close; }
                    if ui.button("Salva con nome").clicked() {
                        let name = std::path::Path::new(&self.name).file_name().and_then(|s| s.to_str()).unwrap_or("file");
                        if let Some(path) = crate::pick::save_as(name) { action = Action::Save(path); }
                    }
                    if self.video && ui.add_enabled(!self.opening, egui::Button::new("Apri")).clicked() {
                        self.opening = true;
                        action = Action::Open;
                    }
                });
                if self.failed { ui.label("Impossibile aprire il contenuto"); }
                if let Some(tex) = self.original.as_ref().or(thumbnail) {
                    let available = ui.available_size().max(egui::Vec2::splat(1.));
                    let image = tex.size_vec2();
                    ui.centered_and_justified(|ui| { ui.add(egui::Image::new(tex).fit_to_exact_size(fit(image, available))); });
                } else if !self.failed { ui.spinner(); }
            });
        action
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fit_keeps_aspect_inside_both_dimensions() {
        use eframe::egui::vec2;
        assert_eq!(super::fit(vec2(400., 200.), vec2(100., 100.)), vec2(100., 50.));
        assert_eq!(super::fit(vec2(200., 400.), vec2(100., 100.)), vec2(50., 100.));
    }
}
