//! Status icon (menu bar on macOS, StatusNotifierItem on Linux) around `tray_menu`'s data model.
//! Menu clicks arrive on the platform's own thread and only go into a channel; the app drains it.
use crate::tray_menu::{TrayAction, TrayItem};
use matrix_domain::AccountId;
use std::sync::mpsc::{Receiver, Sender};
use tray_icon::menu::{IconMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[cfg(target_os = "macos")]
const GLYPH_MAC: &[u8] = include_bytes!("../../../packaging/macos/trayTemplate@2x.png");
// Light glyph for dark panels, dark glyph for light panels.
const GLYPH_LIGHT: &[u8] = include_bytes!("../../../packaging/linux/tray-light@2x.png");
const GLYPH_DARK: &[u8] = include_bytes!("../../../packaging/linux/tray-dark@2x.png");

fn encode(action: TrayAction) -> String {
    match action {
        TrayAction::ToggleWindow => "window".into(),
        TrayAction::ToggleNotifications => "notifications".into(),
        TrayAction::Quit => "quit".into(),
        TrayAction::SwitchAccount(id) => format!("account:{}", id.as_u64()),
    }
}

fn decode(id: &str) -> Option<TrayAction> {
    Some(match id {
        "window" => TrayAction::ToggleWindow,
        "notifications" => TrayAction::ToggleNotifications,
        "quit" => TrayAction::Quit,
        _ => TrayAction::SwitchAccount(AccountId::new(id.strip_prefix("account:")?.parse().ok()?)),
    })
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
    let mut put = |x: usize, y: usize, px: [u8; 4]| rgba[(y * side + x) * 4..][..4].copy_from_slice(&px);
    for y in 0..bh {
        for x in 0..bw {
            let corner = (x == 0 || x == bw - 1) && (y == 0 || y == bh - 1);
            if !corner {
                put(x0 + x, y0 + y, [220, 38, 38, 255]);
            }
        }
    }
    let text_w = text.len() * 3 * scale + (text.len() - 1) * scale;
    let (tx, ty) = (x0 + (bw.saturating_sub(text_w)) / 2, y0 + (bh.saturating_sub(5 * scale)) / 2);
    for (n, ch) in text.bytes().enumerate() {
        let rows = DIGITS[(ch - b'0') as usize];
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    for (dy, dx) in (0..scale).flat_map(|y| (0..scale).map(move |x| (y, x))) {
                        put(tx + (n * 4 + col) * scale + dx, ty + row * scale + dy, [255; 4]);
                    }
                }
            }
        }
    }
}

fn glyph(png: &[u8], unread: u64) -> Option<Icon> {
    let image = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    let mut rgba = image.into_raw();
    if !cfg!(target_os = "macos") {
        draw_badge(&mut rgba, w as usize, unread);
    }
    Icon::from_rgba(rgba, w, h).ok()
}

/// Small round avatar: a plain disc whose colour follows the account (the menu model carries
/// initials only; the picture cache is keyed by mxc, which the catalog does not hold).
fn avatar(id: AccountId) -> Option<tray_icon::menu::Icon> {
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

fn build_menu(items: &[TrayItem]) -> Menu {
    let menu = Menu::new();
    for item in items {
        let _ = match item {
            TrayItem::Separator => menu.append(&PredefinedMenuItem::separator()),
            // muda has no avatar-plus-check on one entry: the shown account gets a "✓" in the
            // title (which assistive tech reads) and keeps its avatar.
            TrayItem::Entry { action, label, checked, .. } => {
                let title = if *checked { format!("✓ {label}") } else { label.clone() };
                let id = encode(*action);
                match action {
                    TrayAction::SwitchAccount(account) => {
                        menu.append(&IconMenuItem::with_id(id, title, true, avatar(*account), None))
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
    menu: Option<Vec<TrayItem>>,
    badge: Option<(u64, bool)>,
}

impl Tray {
    /// None when the platform has no tray host (e.g. no StatusNotifier watcher): the app then
    /// keeps closing its window normally.
    pub fn start() -> Option<Self> {
        let (tx, rx): (Sender<TrayAction>, _) = crate::tray_menu::channel_pair();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(action) = decode(event.id.as_ref()) {
                let _ = tx.send(action);
            }
        }));
        let builder = TrayIconBuilder::new().with_tooltip("Gnobbi");
        let icon = Self::icon(0, false)?;
        // Template images exist on macOS only (they follow the menu bar's light/dark by themselves).
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(icon);
        let built = builder.build();
        match built {
            Ok(icon) => Some(Self { icon, rx, menu: None, badge: None }),
            Err(error) => {
                eprintln!("Gnobbi: no tray icon: {error}");
                None
            }
        }
    }

    fn icon(unread: u64, dark_panel: bool) -> Option<Icon> {
        #[cfg(target_os = "macos")]
        {
            let _ = dark_panel; // the template image follows the menu bar by itself
            glyph(GLYPH_MAC, unread)
        }
        #[cfg(not(target_os = "macos"))]
        glyph(if dark_panel { GLYPH_LIGHT } else { GLYPH_DARK }, unread)
    }

    pub fn actions(&self) -> Vec<TrayAction> {
        crate::tray_menu::drain_tray_actions(&self.rx)
    }

    /// Cheap when nothing changed: the native menu and icon are rebuilt only on a difference.
    pub fn update(&mut self, items: &[TrayItem], unread: u64, dark_panel: bool) {
        if self.menu.as_deref() != Some(items) {
            self.icon.set_menu(Some(Box::new(build_menu(items))));
            self.menu = Some(items.to_vec());
        }
        if self.badge != Some((unread, dark_panel)) {
            if let Some(icon) = Self::icon(unread, dark_panel) {
                #[cfg(target_os = "macos")]
                let _ = self.icon.set_icon_templated(Some(icon));
                #[cfg(not(target_os = "macos"))]
                let _ = self.icon.set_icon(Some(icon));
            }
            // macOS: the count is text next to the icon, absent at 0. Elsewhere it is in the icon.
            if cfg!(target_os = "macos") {
                self.icon.set_title(Some(if unread > 0 { unread.to_string() } else { String::new() }));
            }
            let tip = if unread > 0 { format!("Gnobbi, {unread} non letti") } else { "Gnobbi".into() };
            let _ = self.icon.set_tooltip(Some(tip));
            self.badge = Some((unread, dark_panel));
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
    fn badge_only_touches_the_lower_right_and_only_when_unread() {
        let png = image::load_from_memory(GLYPH_DARK).unwrap().to_rgba8();
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
                    assert_eq!(some[(y * side + x) * 4..][..4], base[(y * side + x) * 4..][..4]);
                }
            }
        }
        // 123 is shown as 99: same pixels as 99.
        let mut capped = base.clone();
        draw_badge(&mut capped, side, 99);
        assert_eq!(some, capped);
    }
}
