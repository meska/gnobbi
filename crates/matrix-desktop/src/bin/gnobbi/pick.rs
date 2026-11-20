//! File pickers: NSOpenPanel / xdg portal / Windows dialog through rfd; on iOS UIDocumentPicker and PHPicker
//! through robius-file-picker, plus the share sheet. The iOS pickers answer later, so the API is
//! `start` (on the click) + `done` (every frame): on desktop `start` blocks and `done` hands the answer back at once.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The last answer, keyed by the title of the button that asked (one dialog at a time).
static DONE: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

fn store(title: &str, path: Option<PathBuf>) {
    *DONE.lock().unwrap_or_else(|e| e.into_inner()) = path.map(|p| (title.to_owned(), p));
}

/// The file chosen for the dialog titled `title`, once; None while open, closed or taken.
pub fn done(title: &str) -> Option<PathBuf> {
    let mut slot = DONE.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|(t, _)| t == title) { slot.take().map(|(_, p)| p) } else { None }
}

// shortcut: blocks the frame loop while the dialog is open (macOS needs the main thread); move to the async API if that is felt.
#[cfg(not(target_os = "ios"))]
pub fn start(_ctx: &eframe::egui::Context, title: &str, images_only: bool) {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if images_only {
        dialog = dialog.add_filter("Immagini", &["png", "jpg", "jpeg", "gif", "webp"]);
    }
    store(title, dialog.pick_file());
}

/// Photos from the library (PHPicker) for images, any file (UIDocumentPicker) otherwise.
#[cfg(target_os = "ios")]
pub fn start(ctx: &eframe::egui::Context, title: &str, images_only: bool) {
    let (ctx, t) = (ctx.clone(), title.to_owned());
    let done = move |r: robius_file_picker::Result<Option<robius_file_picker::PickedFile>>| {
        store(&t, r.ok().flatten().and_then(|f| f.into_path()));
        ctx.request_repaint();
    };
    let dialog = robius_file_picker::FileDialog::new().set_title(title);
    let _ = if images_only { dialog.pick_image(done) } else { dialog.pick_file(done) };
}

/// Where a file to save goes: the folder the user picks on desktop, a private temp file on iOS (shared afterwards).
#[cfg(not(target_os = "ios"))]
pub fn save_as(name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title("Salva con nome").set_file_name(name).save_file()
}

#[cfg(target_os = "ios")]
pub fn save_as(name: &str) -> Option<PathBuf> {
    let path = save_dir().join(safe_name(name));
    let _ = std::fs::remove_file(&path);
    Some(path)
}

/// Where «Scarica» writes: Downloads on desktop, a private temp folder on iOS (the app container has no Downloads the user can reach).
pub fn save_dir() -> PathBuf {
    if cfg!(target_os = "ios") {
        let dir = std::env::temp_dir().join("gnobbi-share");
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        Path::new(&std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()).join("Downloads")
    }
}

/// Only the last component of what the sender wrote, never a path; never empty or hidden.
pub fn safe_name(name: &str) -> String {
    let base = Path::new(name).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let base = base.trim_start_matches('.');
    if base.is_empty() { "file".to_owned() } else { base.to_owned() }
}

/// Opens the share sheet (save to Files, AirDrop, other apps) for a finished file.
#[cfg(target_os = "ios")]
pub fn share(path: &Path) {
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSArray, NSString, NSURL};
    use objc2_ui_kit::{UIActivityViewController, UIApplication};
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(root) = UIApplication::sharedApplication(mtm).keyWindow().and_then(|w| w.rootViewController()) else { return };
    let mut top = root;
    while let Some(next) = top.presentedViewController() { top = next; }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(url))]);
    let sheet = unsafe { UIActivityViewController::initWithActivityItems_applicationActivities(UIActivityViewController::alloc(mtm), &items, None) };
    // iPad shows the sheet as a popover and traps without an anchor.
    if let (Some(pop), Some(view)) = (sheet.popoverPresentationController(), top.view()) {
        let b = view.bounds();
        pop.setSourceView(Some(&view));
        pop.setSourceRect(CGRect::new(CGPoint::new(b.size.width / 2., b.size.height / 2.), CGSize::new(0., 0.)));
    }
    top.presentViewController_animated_completion(&sheet, true, None);
}

#[cfg(not(target_os = "ios"))]
pub fn share(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_goes_to_the_button_that_asked_and_only_once() {
        store("Allega un file", Some("/tmp/a.png".into()));
        assert_eq!(done("Foto della stanza"), None, "another button must not take it");
        assert_eq!(done("Allega un file"), Some("/tmp/a.png".into()));
        assert_eq!(done("Allega un file"), None, "taken once");
        store("Allega un file", None); // dialog closed
        assert_eq!(done("Allega un file"), None);
    }

    #[test]
    fn a_sender_name_never_escapes_the_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name("foto 1.jpg"), "foto 1.jpg");
    }

    #[test]
    fn sender_names_with_trailing_slashes_dots_or_unicode_stay_one_safe_component() {
        assert_eq!(safe_name("dir/"), "dir");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name("a/b/c.txt"), "c.txt");
        assert_eq!(safe_name("résumé 日本語.pdf"), "résumé 日本語.pdf");
    }
}
