//! Native file picker (NSOpenPanel / xdg portal / Windows dialog), the same crate CADCraft uses.
use std::path::PathBuf;

/// One file chosen by the user, or None when the dialog was closed.
// shortcut: blocks the frame loop while the dialog is open (macOS needs the main thread); move to the async API if that is felt.
#[cfg(not(target_os = "ios"))]
pub fn file(title: &str, images_only: bool) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if images_only {
        dialog = dialog.add_filter("Immagini", &["png", "jpg", "jpeg", "gif", "webp"]);
    }
    dialog.pick_file()
}

// iOS stub: no rfd there; the UIDocumentPicker/PHPicker bridge is not written, so attaching a file does nothing.
#[cfg(target_os = "ios")]
pub fn file(_title: &str, _images_only: bool) -> Option<PathBuf> { None }
