use eframe::egui;
use matrix_desktop::i18n;
use matrix_domain::EventId;
use matrix_engine::media::{MediaError, Preview};
use std::path::PathBuf;

/// A picture or video of the chat the viewer can step to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: EventId,
    pub video: bool,
    pub name: String,
}

pub enum Action { None, Close, Save(PathBuf), Open, Go(Item) }

fn fit(image: egui::Vec2, available: egui::Vec2) -> egui::Vec2 {
    image * (available.x / image.x).min(available.y / image.y)
}

/// Farthest a finger or pointer travels before a drag counts as a swipe, in points.
const SWIPE: f32 = 60.;
const SWIPE_CLOSE: f32 = 90.;
const MAX_ZOOM: f32 = 8.;

#[derive(Debug, PartialEq, Eq)]
enum Swipe { None, Next, Previous, Close }

/// What a finished drag of `d` means at normal size: sideways steps to the next or previous picture (left = next), a long
/// vertical one closes (touch only: a mouse drag is not a gesture to dismiss with).
fn swipe(d: egui::Vec2, touch: bool) -> Swipe {
    if d.x.abs() >= SWIPE && d.x.abs() > d.y.abs() {
        if d.x < 0. { Swipe::Next } else { Swipe::Previous }
    } else if touch && d.y.abs() >= SWIPE_CLOSE && d.y.abs() > d.x.abs() {
        Swipe::Close
    } else {
        Swipe::None
    }
}

/// The neighbours of `id` in `items`: (previous, next).
fn around<'a>(items: &'a [Item], id: &EventId) -> (Option<&'a Item>, Option<&'a Item>) {
    match items.iter().position(|i| &i.id == id) {
        Some(at) => (at.checked_sub(1).map(|p| &items[p]), items.get(at + 1)),
        None => (None, None),
    }
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
    /// 1 = the picture fits the screen.
    zoom: f32,
    /// Where the zoomed picture's centre sits, from the middle of the screen.
    pan: egui::Vec2,
    /// How far the current drag has moved the picture at normal size.
    drag: egui::Vec2,
}

impl Viewer {
    pub fn new(id: EventId, video: bool, name: String) -> Self {
        Self { id, video, avatar: false, name, original: None, failed: false, opening: false, player: None, zoom: 1., pan: egui::Vec2::ZERO, drag: egui::Vec2::ZERO }
    }

    /// A profile picture, shown at once from the texture the card already has.
    pub fn for_avatar(name: String, shown: egui::TextureHandle) -> Self {
        Self { avatar: true, original: Some(shown), ..Self::new(EventId::new("avatar"), false, name) }
    }

    /// Shows another picture of the chat from the start: normal size, its own original on the way.
    pub fn go(&mut self, to: &Item) {
        *self = Self::new(to.id.clone(), to.video, to.name.clone());
    }

    /// The viewer is on one of the first pictures of what the chat has loaded: older ones may be asked for.
    pub fn near_start(&self, items: &[Item]) -> bool {
        !self.avatar && items.iter().position(|i| i.id == self.id).is_none_or(|at| at < 3)
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
        self.player = Some(open_external(path, &crate::pick::safe_name(&self.name)));
    }

    /// `items`: the pictures and videos of the chat in order, to step through; `transfer`: how far the video being opened has got.
    pub fn show(&mut self, ctx: &egui::Context, thumbnail: Option<&egui::TextureHandle>, items: &[Item], transfer: Option<(u64, Option<u64>)>) -> Action {
        if let Some(rx) = &self.player && let Ok(ok) = rx.try_recv() {
            self.player = None;
            self.opening = false;
            if ok { return Action::Close; }
            self.failed = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) { return Action::Close; }
        let (previous, next) = if self.avatar { (None, None) } else { around(items, &self.id) };
        let mut action = Action::None;
        let key = |k| ctx.input(|i| i.key_pressed(k));
        if key(egui::Key::ArrowLeft) && let Some(p) = previous { action = Action::Go(p.clone()); }
        if key(egui::Key::ArrowRight) && let Some(n) = next { action = Action::Go(n.clone()); }
        let screen = ctx.content_rect();
        let phone = crate::layout::compact(ctx);
        // The whole window (on a phone the safe area), above everything, on the panel colour.
        let fill = ctx.global_style().visuals.panel_fill;
        egui::Area::new(egui::Id::new("viewer-full")).order(egui::Order::Foreground).fixed_pos(screen.min).constrain(false)
            .show(ctx, |ui| {
                egui::Frame::NONE.fill(fill).show(ui, |ui| {
                ui.set_min_size(screen.size());
                ui.set_max_size(screen.size());
                ui.horizontal(|ui| {
                    let resp = crate::icons::button(ui, crate::icons::Icon::X, false, i18n::t("Close"));
                    if resp.clicked() { action = Action::Close; }
                    if !self.avatar && ui.add(egui::Button::new(i18n::t("Save as")).min_size(egui::Vec2::splat(crate::mobile::target(0.)))).clicked() {
                        let name = std::path::Path::new(&self.name).file_name().and_then(|s| s.to_str()).unwrap_or("file");
                        crate::pick::save_start(ctx, i18n::t("Save as"), name);
                    }
                    if !self.avatar && let Some(path) = crate::pick::done(i18n::t("Save as")) { action = Action::Save(path); }
                    if (self.video || (cfg!(target_os = "android") && !self.avatar)) && ui.add_enabled(!self.opening, egui::Button::new(i18n::t(if cfg!(target_os = "android") { "Share" } else { "Open" })).min_size(egui::Vec2::splat(crate::mobile::target(0.)))).clicked() {
                        self.opening = true;
                        action = Action::Open;
                    }
                    if let Some(at) = items.iter().position(|i| i.id == self.id) && !self.avatar {
                        ui.label(format!("{} / {}", at + 1, items.len()));
                    }
                });
                if self.opening && let Some((got, total)) = transfer {
                    ui.add(egui::ProgressBar::new(crate::media_cards::fraction(got, total).unwrap_or(0.)).desired_height(6.).animate(total.is_none()));
                }
                if self.failed { ui.label(i18n::t("Cannot open the content")); }
                let area = ui.available_rect_before_wrap().intersect(screen); // the Area's own rect is not bounded by the window
                let resp = ui.interact(area, ui.id().with("viewer-area"), egui::Sense::click_and_drag());
                if let Some(tex) = self.original.clone().or_else(|| thumbnail.cloned()) {
                    self.picture(ui, area, &resp, &tex, phone, (previous, next), &mut action);
                } else if !self.failed {
                    ui.put(egui::Rect::from_center_size(area.center(), egui::Vec2::splat(24.)), egui::Spinner::new());
                }
                // Stepping buttons over the picture's sides (the swipe does the same on a phone).
                let reach = crate::mobile::target(44.);
                for (item, side, icon, label) in [(previous, area.left_center() + egui::vec2(reach / 2. + 4., 0.), crate::icons::Icon::CaretLeft, i18n::t("Previous")), (next, area.right_center() - egui::vec2(reach / 2. + 4., 0.), crate::icons::Icon::CaretRight, i18n::t("Next"))] {
                    if let Some(item) = item {
                        let rect = egui::Rect::from_center_size(side, egui::Vec2::splat(reach));
                        if crate::icons::button_at(ui, rect, ui.id().with(("step", label)), icon, false, crate::metrics::get().icon, label, None, true, 1.).clicked() {
                            action = Action::Go(item.clone());
                        }
                    }
                }
                });
            });
        action
    }

    /// The picture with zoom, pan and the drag that steps or closes.
    #[allow(clippy::too_many_arguments)]
    fn picture(&mut self, ui: &mut egui::Ui, area: egui::Rect, resp: &egui::Response, tex: &egui::TextureHandle, phone: bool, (previous, next): (Option<&Item>, Option<&Item>), action: &mut Action) {
        let fitted = fit(tex.size_vec2(), area.size().max(egui::Vec2::splat(1.)));
        // Pinch, and the wheel over the picture: zoom about the centre.
        let (pinch, wheel) = ui.input(|i| (i.zoom_delta(), if resp.hovered() { i.smooth_scroll_delta.y } else { 0. }));
        self.zoom = (self.zoom * pinch * (wheel * 0.0025).exp()).clamp(1., MAX_ZOOM);
        if resp.double_clicked() {
            self.zoom = if self.zoom > 1.01 { 1. } else { 2.5 };
        }
        if self.zoom <= 1.01 {
            self.zoom = 1.;
            self.pan = egui::Vec2::ZERO;
        }
        if resp.dragged() {
            if self.zoom > 1. { self.pan += resp.drag_delta(); } else { self.drag += resp.drag_delta(); }
        }
        let shown = fitted * self.zoom;
        // The zoomed picture cannot be dragged off the screen.
        let room = ((shown - area.size()) / 2.).max(egui::Vec2::ZERO);
        self.pan = self.pan.clamp(-room, room);
        if resp.drag_stopped() {
            let moved = std::mem::take(&mut self.drag);
            if self.zoom == 1. {
                match swipe(moved, phone) {
                    Swipe::Next => if let Some(n) = next { *action = Action::Go(n.clone()); },
                    Swipe::Previous => if let Some(p) = previous { *action = Action::Go(p.clone()); },
                    Swipe::Close => *action = Action::Close,
                    Swipe::None => {}
                }
            }
        }
        let rect = egui::Rect::from_center_size(area.center() + self.pan + if self.zoom == 1. { self.drag } else { egui::Vec2::ZERO }, shown);
        ui.painter().with_clip_rect(area).image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
        if self.drag != egui::Vec2::ZERO { ui.ctx().request_repaint(); }
    }
}

/// Opens `path` with the system's program for it; the answer (did it start) comes on the channel.
/// iOS has no external opener process (and UIKit wants the main thread): the share sheet offers the file to other apps.
/// Android does the same through `Intent.ACTION_SEND`.
pub fn open_external(path: PathBuf, name: &str) -> std::sync::mpsc::Receiver<bool> {
    #[cfg(not(target_os = "android"))]
    let _ = name;
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(target_os = "ios")]
    {
        crate::pick::share(&path);
        let _ = tx.send(true);
        return rx;
    }
    #[cfg(target_os = "android")]
    {
        let _ = tx.send(crate::pick::share_android(&path, name));
        return rx;
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    std::thread::spawn(move || {
        let program = std::env::var("GNOBBI_VIDEO_OPENER").unwrap_or_else(|_| if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" }.into());
        let ok = std::process::Command::new(program).arg(path).status().is_ok_and(|s| s.success());
        let _ = tx.send(ok);
    });
    rx
}

#[derive(Clone, Copy)]
#[allow(dead_code)] // each platform builds only its own arm; the tests build all three
pub enum Os { Mac, Windows, Linux }

/// The command that shows `path` selected in the file manager. `opener` (the prova hook `GNOBBI_VIDEO_OPENER`) replaces the
/// program and gets the file path alone. Linux: only the fallback (the folder in the default file manager); `reveal` asks D-Bus first.
pub fn reveal_command(os: Os, opener: Option<String>, path: &std::path::Path) -> (String, Vec<std::ffi::OsString>) {
    if let Some(program) = opener { return (program, vec![path.into()]); }
    match os {
        Os::Mac => ("open".into(), vec!["-R".into(), path.into()]),
        Os::Windows => {
            // explorer wants the path quoted after the comma (a comma in the name would split it); `reveal` passes it raw.
            let mut arg = std::ffi::OsString::from("/select,\"");
            arg.push(path);
            arg.push("\"");
            ("explorer".into(), vec![arg])
        }
        Os::Linux => ("xdg-open".into(), vec![path.parent().unwrap_or(path).into()]),
    }
}

/// Linux: `org.freedesktop.FileManager1.ShowItems` selects the file in whichever file manager is running.
#[cfg(target_os = "linux")]
fn show_items(path: &std::path::Path) -> bool {
    let Ok(uri) = url::Url::from_file_path(path) else { return false };
    let call = || -> zbus::Result<()> {
        let conn = zbus::blocking::Connection::session()?;
        let proxy = zbus::blocking::Proxy::new(&conn, "org.freedesktop.FileManager1", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1")?;
        proxy.call::<_, _, ()>("ShowItems", &(vec![uri.as_str()], ""))
    };
    call().is_ok()
}

/// Shows the saved `path` in the file manager (desktop); the answer comes on the channel.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn reveal(path: PathBuf) -> std::sync::mpsc::Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let opener = std::env::var("GNOBBI_VIDEO_OPENER").ok();
        #[cfg(target_os = "linux")]
        if opener.is_none() && show_items(&path) { let _ = tx.send(true); return; }
        let os = if cfg!(target_os = "macos") { Os::Mac } else if cfg!(windows) { Os::Windows } else { Os::Linux };
        let (program, args) = reveal_command(os, opener, &path);
        let mut cmd = std::process::Command::new(program);
        #[cfg(windows)]
        {
            // The quoting of `reveal_command` is already final; explorer exits 1 even when it opens the window, so only the start counts.
            use std::os::windows::process::CommandExt;
            for a in &args { cmd.raw_arg(a); }
            let _ = tx.send(cmd.status().is_ok());
        }
        #[cfg(not(windows))]
        let _ = tx.send(cmd.args(args).status().is_ok_and(|s| s.success()));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    #[test]
    fn reveal_selects_the_file_on_each_desktop() {
        let file = std::path::Path::new("/home/a/Downloads/relazione.pdf");
        let (p, a) = reveal_command(Os::Mac, None, file);
        assert_eq!((p.as_str(), a), ("open", vec!["-R".into(), file.into()]));
        let (p, a) = reveal_command(Os::Windows, None, std::path::Path::new(r"C:\Users\a b\relazione, finale.pdf"));
        assert_eq!((p.as_str(), a), ("explorer", vec![r#"/select,"C:\Users\a b\relazione, finale.pdf""#.into()]));
        // Linux without D-Bus: the folder, not the file (xdg-open on the file would run it).
        let (p, a) = reveal_command(Os::Linux, None, file);
        assert_eq!((p.as_str(), a), ("xdg-open", vec!["/home/a/Downloads".into()]));
        // The test hook gets the file path alone, on every platform.
        let (p, a) = reveal_command(Os::Linux, Some("hook".into()), file);
        assert_eq!((p.as_str(), a), ("hook", vec![file.into()]));
    }

    #[test]
    fn fit_keeps_aspect_inside_both_dimensions() {
        assert_eq!(fit(vec2(400., 200.), vec2(100., 100.)), vec2(100., 50.));
        assert_eq!(fit(vec2(200., 400.), vec2(100., 100.)), vec2(50., 100.));
    }

    #[test]
    fn a_sideways_swipe_steps_and_a_long_vertical_one_closes_on_touch_only() {
        assert_eq!(swipe(vec2(-80., 10.), false), Swipe::Next);
        assert_eq!(swipe(vec2(80., -10.), true), Swipe::Previous);
        assert_eq!(swipe(vec2(10., 120.), true), Swipe::Close);
        assert_eq!(swipe(vec2(-10., -120.), true), Swipe::Close);
        assert_eq!(swipe(vec2(10., 120.), false), Swipe::None);
        assert_eq!(swipe(vec2(30., 30.), true), Swipe::None);
        // A diagonal drag goes with its longer side.
        assert_eq!(swipe(vec2(70., 95.), true), Swipe::Close);
        assert_eq!(swipe(vec2(95., 70.), true), Swipe::Previous);
    }

    #[test]
    fn neighbours_follow_the_chat_order_and_stop_at_both_ends() {
        let item = |s: &str| Item { id: EventId::new(s), video: false, name: s.into() };
        let items = [item("$a"), item("$b"), item("$c")];
        assert_eq!(around(&items, &EventId::new("$b")), (Some(&items[0]), Some(&items[2])));
        assert_eq!(around(&items, &EventId::new("$a")), (None, Some(&items[1])));
        assert_eq!(around(&items, &EventId::new("$c")), (Some(&items[1]), None));
        assert_eq!(around(&items, &EventId::new("$zz")), (None, None));
        let v = Viewer::new(EventId::new("$b"), false, String::new());
        assert!(v.near_start(&items));
        assert!(!v.near_start(&[item("$1"), item("$2"), item("$3"), item("$4"), item("$b")]));
    }
}
