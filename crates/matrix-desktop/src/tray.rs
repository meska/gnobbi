//! Status icon (menu bar on macOS, StatusNotifierItem on Linux) around `tray_menu`'s data model.
//! Menu clicks arrive on the platform's own thread and only go into a channel; the app drains it.
use crate::tray_menu::{TrayAction, TrayItem};
use matrix_domain::AccountId;
use std::sync::mpsc::{Receiver, Sender};
use tray_icon::menu::{CheckMenuItem, IconMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[cfg(target_os = "macos")]
const GLYPH_MAC: &[u8] = include_bytes!("../../../packaging/macos/trayTemplate@2x.png");
// SNI hosts do not tell the panel colour (and the app theme says nothing about it): one white
// glyph with a dark outline (`with_outline`) reads on light and dark panels alike.
#[cfg(not(target_os = "macos"))]
const GLYPH_LIGHT: &[u8] = include_bytes!("../../../packaging/linux/tray-light@2x.png");

fn encode(action: TrayAction) -> String {
    match action {
        TrayAction::ToggleWindow => "window".into(),
        TrayAction::ShowWindow => "show".into(),
        TrayAction::ToggleNotifications => "notifications".into(),
        TrayAction::Quit => "quit".into(),
        TrayAction::SwitchAccount(id) => format!("account:{}", id.as_u64()),
        TrayAction::Edit(e) => format!("edit:{}", e.name()),
        TrayAction::Settings => "settings".into(),
    }
}

fn decode(id: &str) -> Option<TrayAction> {
    Some(match id {
        "window" => TrayAction::ToggleWindow,
        "show" => TrayAction::ShowWindow,
        "notifications" => TrayAction::ToggleNotifications,
        "quit" => TrayAction::Quit,
        "settings" => TrayAction::Settings,
        _ if id.starts_with("edit:") => TrayAction::Edit(crate::tray_menu::Edit::from_name(&id[5..])?),
        _ => TrayAction::SwitchAccount(AccountId::new(id.strip_prefix("account:")?.parse().ok()?)),
    })
}

/// Primary click (button released) brings the window to the front; the menu is the secondary one.
fn click_action(event: &TrayIconEvent) -> Option<TrayAction> {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => Some(TrayAction::ShowWindow),
        _ => None,
    }
}

/// Digits 3x5, one row per u8 (bit 2 = left column).
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

/// Draws the unread count (capped at 99) in a red badge at the bottom-right of a square RGBA
/// glyph: the lower-right 12/22 of the canvas, over the speech-bubble tail.
pub(crate) fn draw_badge(rgba: &mut [u8], side: usize, unread: u64) {
    if unread == 0 {
        return;
    }
    let text = unread.min(99).to_string();
    let scale = (side / 22).max(1);
    let (x0, y0) = (side * 12 / 22, side * 12 / 22);
    let (bw, bh) = (side - x0, side - y0);
    let mut put =
        |x: usize, y: usize, px: [u8; 4]| rgba[(y * side + x) * 4..][..4].copy_from_slice(&px);
    for y in 0..bh {
        for x in 0..bw {
            let corner = (x == 0 || x == bw - 1) && (y == 0 || y == bh - 1);
            if !corner {
                put(x0 + x, y0 + y, [220, 38, 38, 255]);
            }
        }
    }
    let text_w = text.len() * 3 * scale + (text.len() - 1) * scale;
    let (tx, ty) = (
        x0 + (bw.saturating_sub(text_w)) / 2,
        y0 + (bh.saturating_sub(5 * scale)) / 2,
    );
    for (n, ch) in text.bytes().enumerate() {
        let rows = DIGITS[(ch - b'0') as usize];
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    for (dy, dx) in (0..scale).flat_map(|y| (0..scale).map(move |x| (y, x))) {
                        put(
                            tx + (n * 4 + col) * scale + dx,
                            ty + row * scale + dy,
                            [255; 4],
                        );
                    }
                }
            }
        }
    }
}

/// Dark opaque ring (2 px) around the opaque pixels, so a white glyph also shows on a light panel.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) fn with_outline(rgba: &mut [u8], side: usize) {
    let opaque = |rgba: &[u8], x: usize, y: usize| rgba[(y * side + x) * 4 + 3] >= 128;
    let src = rgba.to_vec();
    for y in 0..side {
        for x in 0..side {
            if opaque(&src, x, y) {
                continue;
            }
            let near = (y.saturating_sub(2)..(y + 3).min(side))
                .any(|ny| (x.saturating_sub(2)..(x + 3).min(side)).any(|nx| opaque(&src, nx, ny)));
            if near {
                rgba[(y * side + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
}

fn glyph(png: &[u8], unread: u64) -> Option<Icon> {
    let image = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    let mut rgba = image.into_raw();
    if !cfg!(target_os = "macos") {
        with_outline(&mut rgba, w as usize);
        draw_badge(&mut rgba, w as usize, unread);
    }
    Icon::from_rgba(rgba, w, h).ok()
}

/// Round 32 px menu icon from the account's picture; None when it has none (or is empty).
pub(crate) fn round_picture(picture: &image::RgbaImage) -> Option<tray_icon::menu::Icon> {
    const SIDE: u32 = 32;
    if picture.width() == 0 || picture.height() == 0 {
        return None;
    }
    let mut small =
        image::imageops::resize(picture, SIDE, SIDE, image::imageops::FilterType::Triangle);
    for (x, y, px) in small.enumerate_pixels_mut() {
        let (dx, dy) = (
            2 * x as i32 - SIDE as i32 + 1,
            2 * y as i32 - SIDE as i32 + 1,
        );
        if dx * dx + dy * dy > (SIDE * SIDE) as i32 {
            px.0 = [0; 4];
        }
    }
    tray_icon::menu::Icon::from_rgba(small.into_raw(), SIDE, SIDE).ok()
}

type Pictures = Vec<(AccountId, image::RgbaImage)>;

/// The loaded picture of every account that has one; the others are left out (they keep the disc).
pub fn pictures_of<T>(
    rows: &[crate::AccountRow],
    load: impl Fn(&str) -> Option<T>,
) -> Vec<(AccountId, T)> {
    rows.iter()
        .filter_map(|r| Some((r.id, r.picture(&load)?)))
        .collect()
}

/// Fallback when the account has no picture (yet): a plain disc whose colour follows the account.
fn avatar(id: AccountId, pictures: &Pictures) -> Option<tray_icon::menu::Icon> {
    if let Some((_, image)) = pictures.iter().find(|(owner, _)| *owner == id)
        && let Some(icon) = round_picture(image)
    {
        return Some(icon);
    }
    const PALETTE: [[u8; 3]; 6] = [
        [229, 115, 115],
        [240, 160, 60],
        [102, 187, 106],
        [66, 165, 245],
        [149, 117, 205],
        [38, 166, 154],
    ];
    let [r, g, b] = PALETTE[(id.as_u64() % 6) as usize];
    let side = 16i32;
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (2 * x - side + 1, 2 * y - side + 1);
            let inside = dx * dx + dy * dy <= side * side;
            rgba.extend_from_slice(&if inside { [r, g, b, 255] } else { [0; 4] });
        }
    }
    tray_icon::menu::Icon::from_rgba(rgba, 16, 16).ok()
}

fn build_menu(items: &[TrayItem], pictures: &Pictures) -> Menu {
    let menu = Menu::new();
    for item in items {
        let _ = match item {
            TrayItem::Separator => menu.append(&PredefinedMenuItem::separator()),
            // muda has no avatar-plus-check on one entry: the shown account gets a "✓" in the
            // title (which assistive tech reads) and keeps its avatar (every account shows its own).
            TrayItem::Entry {
                action,
                label,
                checked,
                ..
            } => {
                let title = if *checked {
                    format!("✓ {label}")
                } else {
                    label.clone()
                };
                let id = encode(*action);
                match action {
                    TrayAction::SwitchAccount(account) => menu.append(&IconMenuItem::with_id(
                        id,
                        title,
                        true,
                        avatar(*account, pictures),
                        None,
                    )),
                    TrayAction::ToggleNotifications => {
                        menu.append(&CheckMenuItem::with_id(id, label, true, *checked, None))
                    }
                    _ => menu.append(&MenuItem::with_id(id, title, true, None)),
                }
            }
        };
    }
    menu
}

/// macOS menu bar: Gnobbi, Modifica, Finestra. The Edit items are ours, not the system's `copy:`/`paste:`: winit's view does not
/// implement those selectors, and a standard key equivalent would swallow Cmd+C/V before egui. Activated (click or shortcut) they
/// arrive as `TrayAction::Edit`; the shortcut never reaches the view too, so nothing runs twice. Keep the returned `Menu` alive.
#[cfg(target_os = "macos")]
pub fn mac_menubar() -> Menu {
    use crate::tray_menu::Edit;
    use tray_icon::menu::{
        accelerator::{Accelerator, Code, Modifiers},
        Submenu,
    };
    let cmd = |code| Some(Accelerator::new(Modifiers::META, code));
    let item = |id: &str, title: &str, key: Option<Accelerator>| MenuItem::with_id(id, title, true, key);
    let sep = || PredefinedMenuItem::separator();
    let menu = Menu::new();
    use crate::i18n::t_ctx;
    let app = Submenu::new("Gnobbi", true);
    let quit = item("quit", t_ctx("menu", "Quit Gnobbi"), cmd(Code::KeyQ));
    app.append_items(&[
        &item("about", t_ctx("menu", "About Gnobbi"), None),
        &sep(),
        &item("settings", t_ctx("menu", "Settings…"), cmd(Code::Comma)),
        &sep(),
        &PredefinedMenuItem::services(Some(t_ctx("menu", "Services"))),
        &sep(),
        &PredefinedMenuItem::hide(Some(t_ctx("menu", "Hide Gnobbi"))),
        &PredefinedMenuItem::hide_others(Some(t_ctx("menu", "Hide Others"))),
        &PredefinedMenuItem::show_all(Some(t_ctx("menu", "Show All"))),
        &sep(),
        &quit,
    ])
    .ok();
    let edit_item = |e: Edit, title: &str, key: Option<Accelerator>| item(&encode(TrayAction::Edit(e)), title, key);
    let edit = Submenu::new(t_ctx("menu", "Edit"), true);
    edit.append_items(&[
        &edit_item(Edit::Undo, t_ctx("menu", "Undo"), cmd(Code::KeyZ)),
        &edit_item(Edit::Redo, t_ctx("menu", "Redo"), Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyZ))),
        &sep(),
        &edit_item(Edit::Cut, t_ctx("menu", "Cut"), cmd(Code::KeyX)),
        &edit_item(Edit::Copy, t_ctx("menu", "Copy"), cmd(Code::KeyC)),
        &edit_item(Edit::Paste, t_ctx("menu", "Paste"), cmd(Code::KeyV)),
        &edit_item(Edit::SelectAll, t_ctx("menu", "Select All"), cmd(Code::KeyA)),
        &sep(),
        // Not added by the system for a custom Edit menu (only for the standard `paste:` items), so they are explicit.
        &PredefinedMenuItem::start_dictation(Some(t_ctx("menu", "Start Dictation…"))),
        &PredefinedMenuItem::emoji_and_symbols(Some(t_ctx("menu", "Emoji & Symbols"))),
    ])
    .ok();
    let window = Submenu::new(t_ctx("menu", "Window"), true);
    window
        .append_items(&[
            &PredefinedMenuItem::minimize(Some(t_ctx("menu", "Minimize"))),
            &PredefinedMenuItem::maximize(Some(t_ctx("menu", "Zoom"))),
            &sep(),
            &PredefinedMenuItem::bring_all_to_front(Some(t_ctx("menu", "Bring All to Front"))),
        ])
        .ok();
    menu.append_items(&[&app, &edit, &window]).ok();
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();
    menu
}

pub struct Tray {
    icon: TrayIcon,
    rx: Receiver<TrayAction>,
    menu: Option<(Vec<TrayItem>, Pictures)>,
    badge: Option<u64>,
    /// Language of the tooltip; a change forces it to be rebuilt.
    lang: &'static str,
}

impl Tray {
    /// None when the platform has no tray host (e.g. no StatusNotifier watcher): the app then
    /// keeps closing its window normally.
    pub fn start() -> Option<Self> {
        let (tx, rx): (Sender<TrayAction>, _) = crate::tray_menu::channel_pair();
        let click_tx = tx.clone();
        MenuEvent::set_event_handler(Some(std::sync::Arc::new(move |event: MenuEvent| {
            // Menu actions run on the main thread on macOS, where the panel must be opened.
            #[cfg(target_os = "macos")]
            if event.id.as_ref() == "about" {
                return about_panel();
            }
            if let Some(action) = decode(event.id.as_ref()) {
                let _ = tx.send(action);
            }
        })));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let Some(action) = click_action(&event) {
                // A hidden app gets no frames, so nobody would read the action: bring the app back right here.
                #[cfg(target_os = "macos")]
                if matches!(action, TrayAction::ToggleWindow | TrayAction::ShowWindow) {
                    unhide_app();
                }
                let _ = click_tx.send(action);
            }
        }));
        // Linux SNI hosts already show the menu only on the secondary click.
        let builder = TrayIconBuilder::new()
            .with_tooltip("Gnobbi")
            .with_menu_on_left_click(false);
        // Linux SNI: hosts show Title, falling back to the Id ("<pid>-<n>") when it is empty.
        // Not on macOS, where the title is the unread count next to the icon.
        #[cfg(target_os = "linux")]
        let builder = builder.with_id("gnobbi").with_title("Gnobbi");
        let icon = Self::icon(0)?;
        // Template images exist on macOS only (they follow the menu bar's light/dark by themselves).
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon);
        let built = builder.build();
        match built {
            Ok(icon) => Some(Self {
                icon,
                rx,
                menu: None,
                badge: None,
                lang: crate::i18n::current(),
            }),
            Err(error) => {
                eprintln!("Gnobbi: no tray icon: {error}");
                None
            }
        }
    }

    /// Linux: the icon only shows if a StatusNotifierWatcher owns its name on the session bus
    /// (ksni is told to assume it does, so `start` succeeds without one). Elsewhere always.
    pub fn host_present(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            use zbus::blocking::{Connection, fdo::DBusProxy};
            let name = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher");
            Connection::session()
                .ok()
                .zip(name.ok())
                .and_then(|(bus, name)| DBusProxy::new(&bus).ok()?.name_has_owner(name).ok())
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "linux"))]
        true
    }

    fn icon(unread: u64) -> Option<Icon> {
        #[cfg(target_os = "macos")]
        {
            glyph(GLYPH_MAC, unread) // the template image follows the menu bar by itself
        }
        #[cfg(not(target_os = "macos"))]
        glyph(GLYPH_LIGHT, unread)
    }

    pub fn actions(&self) -> Vec<TrayAction> {
        crate::tray_menu::drain_tray_actions(&self.rx)
    }

    /// Forces the next `update` to rebuild the native menu (a check item flips itself on click,
    /// so after a refused toggle the model equals the cache but the widget does not).
    pub fn invalidate_menu(&mut self) {
        self.menu = None;
    }

    /// Cheap when nothing changed: the native menu and icon are rebuilt only on a difference.
    pub fn update(
        &mut self,
        items: &[TrayItem],
        unread: u64,
        pictures: Pictures,
    ) {
        if self.menu.as_ref().map(|(i, p)| (i.as_slice(), p)) != Some((items, &pictures)) {
            self.icon
                .set_menu(Some(Box::new(build_menu(items, &pictures))));
            self.menu = Some((items.to_vec(), pictures));
        }
        if self.lang != crate::i18n::current() {
            self.lang = crate::i18n::current();
            self.badge = None;
        }
        if self.badge != Some(unread) {
            if let Some(icon) = Self::icon(unread) {
                #[cfg(target_os = "macos")]
                let _ = self.icon.set_icon_templated(Some(icon));
                #[cfg(not(target_os = "macos"))]
                let _ = self.icon.set_icon(Some(icon));
            }
            // macOS: the count is text next to the icon, absent at 0. Elsewhere it is in the icon.
            if cfg!(target_os = "macos") {
                self.icon.set_title(Some(if unread > 0 {
                    unread.to_string()
                } else {
                    String::new()
                }));
            }
            let tip = if unread > 0 {
                crate::i18n::plural(unread, "Gnobbi, {n} unread", "Gnobbi, {n} unread")
            } else {
                "Gnobbi".into()
            };
            let _ = self.icon.set_tooltip(Some(tip));
            self.badge = Some(unread);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_credits_lines() {
        let html = about_credits("abc1234", "01/02/2026 03:04", "© 2026 A & B");
        assert!(html.is_ascii());
        let (build, copyright, link) = (html.find(">abc1234 &#183; 01/02/2026 03:04<").unwrap(), html.find("color:#6e6e73\">&#169; 2026 A &#38; B<").unwrap(), html.find("<a href=\"https://gnobbi.chat\">gnobbi.chat</a>").unwrap());
        assert!(build < copyright && copyright < link);
        // Outside a bundle there is no plist value: only the link is left.
        assert_eq!(about_credits("", "", "").matches("<p").count(), 1);
    }

    #[test]
    fn menu_ids_round_trip() {
        for a in [
            TrayAction::ToggleWindow,
            TrayAction::ShowWindow,
            TrayAction::ToggleNotifications,
            TrayAction::Quit,
            TrayAction::SwitchAccount(AccountId::new(7)),
        ] {
            assert_eq!(decode(&encode(a)), Some(a));
        }
        assert_eq!(decode("account:x"), None);
        assert_eq!(decode("other"), None);
    }

    #[test]
    fn only_primary_release_shows_the_window() {
        let click = |button, button_state| TrayIconEvent::Click {
            id: Default::default(),
            position: Default::default(),
            rect: Default::default(),
            button,
            button_state,
        };
        assert_eq!(
            click_action(&click(MouseButton::Left, MouseButtonState::Up)),
            Some(TrayAction::ShowWindow)
        );
        assert_eq!(click_action(&click(MouseButton::Left, MouseButtonState::Down)), None);
        assert_eq!(click_action(&click(MouseButton::Right, MouseButtonState::Up)), None);
        assert_eq!(click_action(&click(MouseButton::Middle, MouseButtonState::Up)), None);
    }

    #[test]
    fn outline_rings_the_glyph_and_leaves_it_intact() {
        let side = 8;
        let mut px = vec![0u8; side * side * 4];
        px[(4 * side + 4) * 4..][..4].copy_from_slice(&[255; 4]);
        let mut out = px.clone();
        with_outline(&mut out, side);
        assert_eq!(out[(4 * side + 4) * 4..][..4], [255; 4]);
        // 2 px ring: dark and opaque next to the glyph, untouched farther out.
        assert_eq!(out[(4 * side + 6) * 4..][..4], [0, 0, 0, 255]);
        assert_eq!(out[(2 * side + 2) * 4..][..4], [0, 0, 0, 255]);
        assert_eq!(out[7 * 4..][..4], [0; 4]);
    }

    #[test]
    fn shipped_glyph_has_a_dark_edge_and_a_light_body() {
        #[cfg(not(target_os = "macos"))]
        {
            let png = image::load_from_memory(GLYPH_LIGHT).unwrap().to_rgba8();
            let side = png.width() as usize;
            let mut rgba = png.into_raw();
            with_outline(&mut rgba, side);
            let lum = |p: &[u8]| p[..3].iter().map(|&c| c as u32).sum::<u32>() / 3;
            let seen: Vec<_> = rgba.chunks(4).filter(|p| p[3] == 255).collect();
            assert!(
                seen.iter().any(|p| lum(p) >= 240),
                "light body for dark panels"
            );
            assert!(
                seen.iter().any(|p| lum(p) <= 15),
                "dark edge for light panels"
            );
        }
    }

    #[test]
    fn account_picture_becomes_a_round_icon() {
        let img = image::RgbaImage::from_pixel(96, 96, image::Rgba([10, 200, 30, 255]));
        assert!(round_picture(&img).is_some());
        assert!(round_picture(&image::RgbaImage::new(0, 0)).is_none());
    }

    #[test]
    fn every_account_with_a_loaded_picture_gets_one() {
        let row = |id, avatar: Option<&str>| crate::AccountRow {
            id: AccountId::new(id),
            name: String::new(),
            user_id: String::new(),
            initials: String::new(),
            avatar: avatar.map(str::to_owned),
            current: id == 1,
            unread: 0,
            mentions: 0,
        };
        let rows = [
            row(1, Some("mxc://s/a")),
            row(2, Some("mxc://s/b")),
            row(3, Some("mxc://s/not-cached")),
            row(4, None),
        ];
        let cached = |m: &str| (m != "mxc://s/not-cached").then_some(m.len());
        let got = pictures_of(&rows, cached);
        assert_eq!(got, vec![(AccountId::new(1), 9), (AccountId::new(2), 9)]);
    }

    #[test]
    fn badge_only_touches_the_lower_right_and_only_when_unread() {
        #[cfg(target_os = "macos")]
        let glyph = GLYPH_MAC;
        #[cfg(not(target_os = "macos"))]
        let glyph = GLYPH_LIGHT;
        let png = image::load_from_memory(glyph).unwrap().to_rgba8();
        let side = png.width() as usize;
        let base = png.into_raw();
        let mut none = base.clone();
        draw_badge(&mut none, side, 0);
        assert_eq!(none, base);
        let mut some = base.clone();
        draw_badge(&mut some, side, 123);
        assert_ne!(some, base);
        let from = side * 12 / 22;
        for y in 0..side {
            for x in 0..side {
                if x < from || y < from {
                    assert_eq!(
                        some[(y * side + x) * 4..][..4],
                        base[(y * side + x) * 4..][..4]
                    );
                }
            }
        }
        // 123 is shown as 99: same pixels as 99.
        let mut capped = base.clone();
        draw_badge(&mut capped, side, 99);
        assert_eq!(some, capped);
    }
}

/// Credits of the macOS About panel, under the version: build line, dimmed copyright, site link; an empty value drops its line.
/// ASCII only (entities), since the HTML reader assumes no charset.
#[cfg(any(target_os = "macos", test))]
fn about_credits(commit: &str, built: &str, copyright: &str) -> String {
    let esc = |s: &str| s.chars().map(|c| if c.is_ascii() && !"&<>".contains(c) { c.to_string() } else { format!("&#{};", c as u32) }).collect::<String>();
    let build = [commit, built].iter().filter(|s| !s.is_empty()).map(|s| esc(s)).collect::<Vec<_>>().join(" &#183; ");
    let mut html = String::from(r#"<div style="text-align:center;font:11px -apple-system">"#);
    for (text, color) in [(build, ""), (esc(copyright), ";color:#6e6e73")] {
        if !text.is_empty() {
            html.push_str(&format!(r#"<p style="margin:0 0 4px{color}">{text}</p>"#));
        }
    }
    html + r#"<p style="margin:0 0 8px"><a href="https://gnobbi.chat">gnobbi.chat</a></p></div>"#
}

/// macOS: the standard About panel with our credits. Everything is read from the bundle's Info.plist (packaging/macos/info-plist.sh),
/// so a bundle needs no other file; NSHumanReadableCopyright is shown inside the credits, dimmed and above the link.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc 0.2 macros test a `cargo-clippy` feature
fn about_panel() {
    use cocoa::{appkit::NSApp, base::{id, nil}, foundation::NSString};
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: called from the menu item's main-thread action with a live NSApplication; every object is autoreleased or owned by
    // the bundle, the strings are copied before the pool drains.
    unsafe {
        let ns = |s: &str| -> id { msg_send![NSString::alloc(nil).init_str(s), autorelease] };
        let bundle: id = msg_send![class!(NSBundle), mainBundle];
        let info = |key: &str| -> String {
            let v: id = msg_send![bundle, objectForInfoDictionaryKey: ns(key)];
            let is_str: bool = v != nil && msg_send![v, isKindOfClass: class!(NSString)];
            if !is_str {
                return String::new();
            }
            let c: *const std::os::raw::c_char = msg_send![v, UTF8String];
            std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()
        };
        let html = about_credits(&info("GnobbiCommit"), &info("GnobbiBuilt"), &info("NSHumanReadableCopyright"));
        let data: id = msg_send![class!(NSData), dataWithBytes: html.as_ptr() length: html.len()];
        let credits: id = msg_send![class!(NSAttributedString), alloc];
        let credits: id = msg_send![credits, initWithHTML: data documentAttributes: nil];
        if credits == nil {
            let _: () = msg_send![NSApp(), orderFrontStandardAboutPanel: nil];
            return;
        }
        let credits: id = msg_send![credits, autorelease];
        // An empty "Copyright" keeps the panel from drawing the plist line a second time, last and at full colour.
        let (objects, keys) = ([credits, ns("")], [ns("Credits"), ns("Copyright")]);
        let options: id = msg_send![class!(NSDictionary), dictionaryWithObjects: objects.as_ptr() forKeys: keys.as_ptr() count: 2usize];
        let _: () = msg_send![NSApp(), orderFrontStandardAboutPanelWithOptions: options];
    }
}

/// macOS: unhide and activate the app (the tray click runs on the main thread, through the status item's action).
#[cfg(target_os = "macos")]
fn unhide_app() {
    use cocoa::{appkit::NSApp, base::{id, nil, YES}};
    use objc::{msg_send, sel, sel_impl};
    // SAFETY: NSApp() is the live shared application; both messages are plain calls made from the status item's main-thread action.
    unsafe {
        let app: id = NSApp();
        let _: () = msg_send![app, unhide: nil];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}
