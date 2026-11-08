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
    }
}

fn decode(id: &str) -> Option<TrayAction> {
    Some(match id {
        "window" => TrayAction::ToggleWindow,
        "show" => TrayAction::ShowWindow,
        "notifications" => TrayAction::ToggleNotifications,
        "quit" => TrayAction::Quit,
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
    if let Some((_, image)) = pictures.iter().find(|(owner, _)| *owner == id) {
        if let Some(icon) = round_picture(image) {
            return Some(icon);
        }
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

pub struct Tray {
    icon: TrayIcon,
    rx: Receiver<TrayAction>,
    menu: Option<(Vec<TrayItem>, Pictures)>,
    badge: Option<u64>,
}

impl Tray {
    /// None when the platform has no tray host (e.g. no StatusNotifier watcher): the app then
    /// keeps closing its window normally.
    pub fn start() -> Option<Self> {
        let (tx, rx): (Sender<TrayAction>, _) = crate::tray_menu::channel_pair();
        let click_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(action) = decode(event.id.as_ref()) {
                let _ = tx.send(action);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let Some(action) = click_action(&event) {
                let _ = click_tx.send(action);
            }
        }));
        // Linux SNI hosts already show the menu only on the secondary click.
        let builder = TrayIconBuilder::new()
            .with_tooltip("Gnobbi")
            .with_menu_on_left_click(false);
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
            }),
            Err(error) => {
                eprintln!("Gnobbi: no tray icon: {error}");
                None
            }
        }
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
                format!("Gnobbi, {unread} non letti")
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
        let cached = |m: &str| (m != "mxc://s/not-cached").then(|| m.len());
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
