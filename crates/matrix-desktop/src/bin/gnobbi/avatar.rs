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

/// Above this many member pictures the oldest ones that are not on screen are dropped (a room of 8000 would hold ~300 MB of textures);
/// the engine's byte cache makes asking for them again cheap. `queue` lists the member pictures asked for, oldest first;
/// pictures of other things (rooms, stickers, timeline senders) are never counted nor dropped here. shortcut: no LRU, a picture is not refreshed when seen again.
pub const MEMBER_CAP: usize = 300;

/// A dropped picture still downloading loses its `Loading` mark too: `Session::drive_avatars` discards an answer nobody waits for.
pub fn trim_members(avatars: &mut Avatars, queue: &mut Vec<String>, keep: &[String]) {
    let mut excess = queue.len().saturating_sub(MEMBER_CAP);
    queue.retain(|m| {
        if excess == 0 || keep.contains(m) {
            return true;
        }
        excess -= 1;
        if matches!(avatars.get(m), Some(Avatar::Ready(_) | Avatar::Loading)) {
            avatars.remove(m);
        }
        false
    });
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

    fn ready(n: usize) -> (Vec<String>, Avatars) {
        let ctx = egui::Context::default();
        let tex = ctx.load_texture("t", egui::ColorImage::example(), Default::default());
        let names: Vec<String> = (0..n).map(|i| format!("mxc://x/{i}")).collect();
        (names.clone(), names.into_iter().map(|n| (n, Avatar::Ready(tex.clone()))).collect())
    }

    #[test]
    fn member_pictures_stay_under_the_cap_and_the_visible_ones_stay() {
        let (names, all) = ready(2000);
        let (mut map, mut queue) = (Avatars::new(), Vec::new());
        let keep = vec![names[0].clone()];
        for n in &names {
            map.insert(n.clone(), all[n].clone());
            queue.push(n.clone());
            // What the session does after every batch while scrolling.
            trim_members(&mut map, &mut queue, &keep);
            assert!(map.len() <= MEMBER_CAP + 1 && queue.len() <= MEMBER_CAP + 1, "{}", map.len());
        }
        assert!(map.contains_key(&keep[0]));
    }

    #[test]
    fn other_pictures_neither_count_nor_get_dropped() {
        let (names, mut map) = ready(MEMBER_CAP + 50);
        let (members, others) = names.split_at(10);
        let mut queue = members.to_vec();
        for _ in 0..5 {
            trim_members(&mut map, &mut queue, &[]);
        }
        // Over the cap in the map, under it in members: everything stays, senders of the timeline included.
        assert_eq!((map.len(), queue.len(), others.iter().all(|o| map.contains_key(o))), (MEMBER_CAP + 50, 10, true));
    }

    #[test]
    fn an_evicted_loading_picture_cannot_come_back_when_it_arrives() {
        let (names, _) = ready(MEMBER_CAP + 5);
        let mut map: Avatars = names.iter().map(|n| (n.clone(), Avatar::Loading)).collect();
        let mut queue = names.clone();
        trim_members(&mut map, &mut queue, &[]);
        assert_eq!((queue.len(), map.len()), (MEMBER_CAP, MEMBER_CAP));
        // The answer of an evicted one finds no entry: the session drops it (`contains_key`), so the map stays at the cap.
        assert!(!map.contains_key(&names[0]) && map.contains_key(&names[MEMBER_CAP + 4]));
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
