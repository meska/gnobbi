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

/// Same for the member rows on screen (preview and members page).
pub const MEMBER_AVATARS: &str = "member-avatars";

pub type Avatars = std::collections::HashMap<String, Avatar>;

/// Above this many entries the member pictures that are not on screen are dropped (a room of 8000 would hold ~300 MB of textures);
/// the engine's byte cache makes asking for them again cheap. shortcut: no LRU, the ones on screen are simply kept.
pub const MEMBER_CAP: usize = 300;

/// Drops the pictures of `members` that are not in `keep` once the map is over `MEMBER_CAP`.
pub fn trim_members<'a>(avatars: &mut Avatars, members: impl Iterator<Item = &'a str>, keep: &[String]) {
    if avatars.len() <= MEMBER_CAP {
        return;
    }
    for m in members.filter(|m| !keep.iter().any(|k| k == m)) {
        if matches!(avatars.get(m), Some(Avatar::Ready(_))) {
            avatars.remove(m);
        }
    }
}

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
    fn member_pictures_stay_under_the_cap_and_the_visible_ones_stay() {
        let ctx = egui::Context::default();
        let tex = ctx.load_texture("t", egui::ColorImage::example(), Default::default());
        let names: Vec<String> = (0..2000).map(|i| format!("mxc://x/{i}")).collect();
        let mut map = Avatars::new();
        let keep = vec![names[1999].clone()];
        for (i, n) in names.iter().enumerate() {
            map.insert(n.clone(), Avatar::Ready(tex.clone()));
            // What the session does after every batch while scrolling.
            trim_members(&mut map, names[..=i].iter().map(String::as_str), &keep);
            assert!(map.len() <= MEMBER_CAP + 1, "{}", map.len());
        }
        assert!(map.contains_key(&keep[0]));
    }

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
