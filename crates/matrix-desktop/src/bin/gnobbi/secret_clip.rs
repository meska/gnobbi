//! Clipboard for a secret: marked sensitive where the OS has a flag (macOS, Windows, Linux), and wiped
//! after `CLIPBOARD_SECONDS` if it still holds the same text. Only a hash of the text is kept.
//! shortcut: the wipe runs from the frame loop, so it waits for a frame if the window is hidden; iOS only copies.
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static PENDING: Mutex<Option<(Instant, u64)>> = Mutex::new(None);

fn hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

#[cfg(not(target_os = "ios"))]
pub fn copy(ctx: &eframe::egui::Context, text: &str) {
    let mut board = match arboard::Clipboard::new() {
        Ok(b) => b,
        Err(_) => return ctx.copy_text(text.to_owned()),
    };
    let set = board.set();
    #[cfg(target_os = "macos")]
    let set = arboard::SetExtApple::exclude_from_history(set);
    #[cfg(target_os = "windows")]
    let set = arboard::SetExtWindows::exclude_from_history(arboard::SetExtWindows::exclude_from_cloud(set));
    #[cfg(target_os = "linux")]
    let set = arboard::SetExtLinux::exclude_from_history(set);
    if set.text(text.to_owned()).is_err() {
        return ctx.copy_text(text.to_owned());
    }
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now() + Duration::from_secs(matrix_desktop::key_export::CLIPBOARD_SECONDS), hash(text)));
    ctx.request_repaint_after(Duration::from_secs(matrix_desktop::key_export::CLIPBOARD_SECONDS));
}

#[cfg(target_os = "ios")]
pub fn copy(ctx: &eframe::egui::Context, text: &str) {
    ctx.copy_text(text.to_owned());
}

/// Every frame: wipes the clipboard once the delay is over and the text is still ours.
#[cfg(not(target_os = "ios"))]
pub fn tick(ctx: &eframe::egui::Context) {
    let mut slot = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let Some((due, h)) = *slot else { return };
    if Instant::now() < due {
        ctx.request_repaint_after(due - Instant::now());
        return;
    }
    *slot = None;
    if let Ok(mut board) = arboard::Clipboard::new()
        && board.get_text().is_ok_and(|t| hash(&t) == h)
    {
        let _ = board.clear();
    }
}

#[cfg(target_os = "ios")]
pub fn tick(_ctx: &eframe::egui::Context) {}
