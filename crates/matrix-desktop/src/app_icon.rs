//! Runtime app icon: the chosen variant becomes the Dock icon (macOS) or the window/taskbar
//! icon (Windows). Linux/Wayland has no runtime API: the icon comes from the installed desktop entry.
use crate::IconChoice;

impl IconChoice {
    /// 512 px PNG used for the Dock / taskbar icon (macOS: squircle with template margin and alpha).
    pub fn runtime_png(self) -> &'static [u8] {
        #[cfg(target_os = "macos")]
        return match self {
            IconChoice::MinimalMark => include_bytes!("../assets/icon-segno-minimo-mac-512.png"),
            IconChoice::Gnome => include_bytes!("../assets/icon-gnomo-mac-512.png"),
            IconChoice::Monster => include_bytes!("../assets/icon-mostriciattolo-mac-512.png"),
        };
        #[cfg(not(target_os = "macos"))]
        match self {
            IconChoice::MinimalMark => include_bytes!("../assets/icon-segno-minimo-512.png"),
            IconChoice::Gnome => include_bytes!("../assets/icon-gnomo-512.png"),
            IconChoice::Monster => include_bytes!("../assets/icon-mostriciattolo-512.png"),
        }
    }
}

impl IconChoice {
    /// 256 px PNG for the Linux icon theme (the 512 px one is `runtime_png`).
    pub fn theme_png_256(self) -> &'static [u8] {
        match self {
            IconChoice::MinimalMark => include_bytes!("../assets/icon-segno-minimo-256.png"),
            IconChoice::Gnome => include_bytes!("../assets/icon-gnomo-256.png"),
            IconChoice::Monster => include_bytes!("../assets/icon-mostriciattolo-256.png"),
        }
    }
}

/// Whether this OS can switch the icon of a running app.
pub const fn runtime_switch_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Applies `icon` now; a failure leaves the previous icon (cosmetic, never an error to the user).
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub fn apply(icon: IconChoice, _window: &gpui::Window) {
    use cocoa::{
        appkit::{NSApp, NSImage},
        base::{id, nil},
        foundation::NSData,
    };
    use objc::{msg_send, sel, sel_impl};
    let png = icon.runtime_png();
    // SAFETY: called from GPUI callbacks, i.e. on the main thread, with a live NSApplication.
    unsafe {
        let data = NSData::dataWithBytes_length_(nil, png.as_ptr().cast(), png.len() as u64);
        let image: id = NSImage::initWithData_(NSImage::alloc(nil), data);
        if image != nil {
            let _: () = msg_send![NSApp(), setApplicationIconImage: image];
        }
    }
}

#[cfg(all(feature = "desktop", target_os = "windows"))]
pub fn apply(icon: IconChoice, window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            CreateIconFromResourceEx, GetSystemMetrics, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR,
            SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON,
        },
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
        // SAFETY: plain Win32 calls on the UI thread; the HICON is handed to the window, which keeps it.
        unsafe {
            let size = GetSystemMetrics(metric);
            // shortcut: the previous HICON is not destroyed (a few KB per switch), fine for a manual setting.
            if let Ok(hicon) = CreateIconFromResourceEx(icon.runtime_png(), true, 0x0003_0000, size, size, LR_DEFAULTCOLOR) {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(hicon.0 as isize)));
            }
        }
    }
}

/// Linux/Wayland: nothing to do at runtime.
#[cfg(all(feature = "desktop", not(any(target_os = "macos", target_os = "windows"))))]
pub fn apply(_icon: IconChoice, _window: &gpui::Window) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_maps_to_its_own_512_png() {
        let mut seen = std::collections::HashSet::new();
        for icon in IconChoice::ALL {
            let png = icon.runtime_png();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{icon:?}");
            assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 512, "{icon:?} width");
            assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 512, "{icon:?} height");
            assert!(seen.insert(png.as_ptr()), "{icon:?} shares a resource");
            assert_ne!(png, icon.thumbnail());
        }
    }

    #[test]
    fn runtime_switch_only_on_macos_and_windows() {
        assert_eq!(runtime_switch_supported(), cfg!(any(target_os = "macos", target_os = "windows")));
    }
}
