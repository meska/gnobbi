//! The one way to draw a round avatar: solid theme-coloured circle always, the picture over it,
//! initials only when there is no picture (none, or the download failed).
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

/// What the window knows about one avatar picture.
#[derive(Clone)]
pub enum Avatar {
    /// Requested, not arrived: circle without letters (no flash of initials).
    Loading,
    Ready(egui::TextureHandle),
    /// No picture, or it could not be downloaded.
    Missing,
}

/// Context-data key under which the composer lists the mention suggestions on screen; `drive_avatars` asks only for those.
pub const MENTION_AVATARS: &str = "mention-avatars";

pub type Avatars = std::collections::HashMap<String, Avatar>;

impl Avatar {
    fn initials_shown(&self) -> bool {
        matches!(self, Avatar::Missing)
    }
}

/// State of `mxc`: no reference = missing; not in the map yet = about to be requested = loading.
pub fn lookup(avatars: &Avatars, mxc: Option<&str>) -> Avatar {
    mxc.map_or(Avatar::Missing, |m| avatars.get(m).cloned().unwrap_or(Avatar::Loading))
}

/// Circle of `size` centred at `c` in `color`, then the picture, or the initials in `ink`.
pub fn paint(ui: &egui::Ui, c: Pos2, size: f32, color: Color32, state: &Avatar, initials: &str, font: f32, ink: Color32) {
    ui.painter().circle_filled(c, size / 2., color);
    if let Avatar::Ready(tex) = state {
        let rect = Rect::from_center_size(c, Vec2::splat(size));
        egui::Image::new(tex).fit_to_exact_size(rect.size()).corner_radius(egui::CornerRadius::same((size / 2.) as u8)).paint_at(ui, rect);
    } else if state.initials_shown() {
        ui.painter().text(c, egui::Align2::CENTER_CENTER, initials, egui::FontId::proportional(font), ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_only_without_picture() {
        let ctx = egui::Context::default();
        let tex = ctx.load_texture("t", egui::ColorImage::example(), Default::default());
        assert!(!Avatar::Loading.initials_shown());
        assert!(!Avatar::Ready(tex.clone()).initials_shown());
        assert!(Avatar::Missing.initials_shown());
        let mut map = Avatars::new();
        map.insert("a".into(), Avatar::Ready(tex));
        map.insert("b".into(), Avatar::Missing);
        assert!(matches!(lookup(&map, Some("a")), Avatar::Ready(_)));
        assert!(matches!(lookup(&map, Some("b")), Avatar::Missing));
        assert!(matches!(lookup(&map, Some("c")), Avatar::Loading));
        assert!(matches!(lookup(&map, None), Avatar::Missing));
    }
}
