use eframe::egui;
use matrix_desktop::i18n;
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
    /// A profile picture: not an event's media, so nothing to save or open.
    avatar: bool,
    name: String,
    original: Option<egui::TextureHandle>,
    pub failed: bool,
    pub opening: bool,
    player: Option<std::sync::mpsc::Receiver<bool>>,
}

impl Viewer {
    pub fn new(id: EventId, video: bool, name: String) -> Self {
        Self { id, video, avatar: false, name, original: None, failed: false, opening: false, player: None }
    }

    /// A profile picture, shown at once from the texture the card already has.
    pub fn for_avatar(name: String, shown: egui::TextureHandle) -> Self {
        Self { avatar: true, original: Some(shown), ..Self::new(EventId::new("avatar"), false, name) }
    }

    /// The picture as uploaded, when it arrives.
    pub fn set_avatar(&mut self, ctx: &egui::Context, (w, h, rgba): (u32, u32, Vec<u8>)) {
        self.original = Some(ctx.load_texture("avatar-original", egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba), Default::default()));
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
        // iOS has no external opener process (and UIKit wants the main thread): the share sheet offers the file to other apps.
        #[cfg(target_os = "ios")]
        {
            crate::pick::share(&path);
            let _ = tx.send(true);
            return;
        }
        #[cfg(not(target_os = "ios"))]
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
        let screen = ctx.content_rect();
        let phone = crate::mobile::touch();
        let window = egui::Window::new(if self.video { i18n::t(i18n::t("Video")) } else { i18n::t("Image") }).collapsible(false);
        // Phone: the whole safe area, no title bar to drag; desktop: a resizable window.
        let window = if phone {
            window.title_bar(false).resizable(false).fixed_rect(screen)
        } else {
            window.resizable(true).default_size(screen.size() * 0.8).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        };
        window
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let resp = crate::icons::button(ui, crate::icons::Icon::X, false, matrix_desktop::i18n::t("Close"));
                    if resp.clicked() { action = Action::Close; }
                    if !self.avatar && ui.add(egui::Button::new(i18n::t("Save as")).min_size(egui::Vec2::splat(crate::mobile::target(0.)))).clicked() {
                        let name = std::path::Path::new(&self.name).file_name().and_then(|s| s.to_str()).unwrap_or("file");
                        crate::pick::save_start(ctx, i18n::t("Save as"), name);
                    }
                    if !self.avatar && let Some(path) = crate::pick::done(i18n::t("Save as")) { action = Action::Save(path); }
                    if self.video && ui.add_enabled(!self.opening, egui::Button::new(i18n::t("Open"))).clicked() {
                        self.opening = true;
                        action = Action::Open;
                    }
                });
                if self.failed { ui.label(i18n::t("Cannot open the content")); }
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
