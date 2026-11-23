//! File pickers: NSOpenPanel / xdg portal / Windows dialog through rfd; on iOS UIDocumentPicker and PHPicker
//! through robius-file-picker, plus the share sheet. The iOS pickers answer later, so the API is
//! `start` (on the click) + `done` (every frame): on desktop rfd's async dialog runs beside the frame (a blocking
//! one nests a modal loop inside the winit callback, and AppKit events re-entering it abort the process).
use matrix_desktop::i18n;
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

/// Runs an rfd dialog future off the frame and files its answer. The future is created by the caller on the
/// main thread (macOS opens the sheet right there); the sheet blocks the window, so one dialog at a time.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn run(ctx: &eframe::egui::Context, title: &str, dialog: impl std::future::Future<Output = Option<rfd::FileHandle>> + Send + 'static) {
    let (ctx, title) = (ctx.clone(), title.to_owned());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("tokio runtime");
        store(&title, rt.block_on(dialog).map(|h| h.path().to_path_buf()));
        ctx.request_repaint();
    });
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn start(ctx: &eframe::egui::Context, title: &str, images_only: bool) {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if images_only {
        dialog = dialog.add_filter(i18n::t("Images"), &["png", "jpg", "jpeg", "gif", "webp"]);
    }
    run(ctx, title, dialog.pick_file());
}

// Android: no picker yet (SAF intent needed through JNI); the attach button answers "nothing picked".
#[cfg(target_os = "android")]
pub fn start(_ctx: &eframe::egui::Context, title: &str, _images_only: bool) {
    store(title, None);
}

#[cfg(target_os = "android")]
pub fn save_start(_ctx: &eframe::egui::Context, key: &str, _name: &str) {
    store(key, None);
}

/// Photos from the library (PHPicker) for images, any file (UIDocumentPicker) otherwise.
#[cfg(target_os = "ios")]
pub fn start(ctx: &eframe::egui::Context, title: &str, images_only: bool) {
    purge_temp(); // the previous pick has been read by now
    let (ctx, t) = (ctx.clone(), title.to_owned());
    let done = move |r: robius_file_picker::Result<Option<robius_file_picker::PickedFile>>| {
        store(&t, r.ok().flatten().and_then(|f| f.into_path()));
        ctx.request_repaint();
    };
    let dialog = robius_file_picker::FileDialog::new().set_title(title);
    let _ = if images_only { dialog.pick_image(done) } else { dialog.pick_file(done) };
}

/// iOS: picked copies (robius `into_path` drops their cleanup) and shared files stay in tmp until iOS purges it;
/// delete them at the next pick/save and at launch. shortcut: not right after upload/share, upgrade with a per-file guard.
#[cfg(target_os = "ios")]
pub fn purge_temp() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name == "gnobbi-share" || name.starts_with("robius-file-picker-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Where a file to save goes: the file the user picks on desktop, a private temp file on iOS (shared afterwards).
/// The answer comes from `done(key)`.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn save_start(ctx: &eframe::egui::Context, key: &str, name: &str) {
    run(ctx, key, rfd::AsyncFileDialog::new().set_title(i18n::t("Save as")).set_file_name(name).save_file());
}

#[cfg(target_os = "ios")]
pub fn save_start(_ctx: &eframe::egui::Context, key: &str, name: &str) {
    let path = save_dir().join(safe_name(name));
    let _ = std::fs::remove_file(&path);
    store(key, Some(path));
}

/// Where «Scarica» writes: Downloads on desktop, a private temp folder on iOS (the app container has no Downloads the user can reach).
pub fn save_dir() -> PathBuf {
    if cfg!(target_os = "ios") {
        #[cfg(target_os = "ios")]
        purge_temp();
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

/// How the last share sheet ended: None while open, then Some(Some(true)) an action finished, Some(Some(false)) it failed, Some(None) cancelled.
static SHARED: Mutex<Option<Option<bool>>> = Mutex::new(None);

/// The outcome of the share sheet, once it is closed.
pub fn shared() -> Option<Option<bool>> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Opens the share sheet (save to Files, AirDrop, other apps) for a finished file; `shared` tells how it went.
#[cfg(target_os = "ios")]
pub fn share(path: &Path) {
    use block2::RcBlock;
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, runtime::{AnyObject, Bool}};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSArray, NSError, NSString, NSURL};
    use objc2_ui_kit::{UIActivityType, UIActivityViewController, UIApplication, UIDevice, UIUserInterfaceIdiom};
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(root) = UIApplication::sharedApplication(mtm).keyWindow().and_then(|w| w.rootViewController()) else { return };
    let mut top = root;
    while let Some(next) = top.presentedViewController() { top = next; }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(url))]);
    let sheet = unsafe { UIActivityViewController::initWithActivityItems_applicationActivities(UIActivityViewController::alloc(mtm), &items, None) };
    *SHARED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let done = RcBlock::new(|_: *mut UIActivityType, completed: Bool, _: *mut NSArray, error: *mut NSError| {
        *SHARED.lock().unwrap_or_else(|e| e.into_inner()) = Some(if !error.is_null() { Some(false) } else { completed.as_bool().then_some(true) });
    });
    unsafe { sheet.setCompletionWithItemsHandler(&*done as *const _ as *mut _) };
    // Only the iPad shows the sheet as a popover (and traps without an anchor); on the iPhone an anchor displaces the standard bottom sheet.
    if UIDevice::currentDevice(mtm).userInterfaceIdiom() == UIUserInterfaceIdiom::Pad
        && let (Some(pop), Some(view)) = (sheet.popoverPresentationController(), top.view())
    {
        let b = view.bounds();
        pop.setSourceView(Some(&view));
        pop.setSourceRect(CGRect::new(CGPoint::new(b.size.width / 2., b.size.height / 2.), CGSize::new(0., 0.)));
    }
    top.presentViewController_animated_completion(&sheet, true, None);
}

#[cfg(not(target_os = "ios"))]
pub fn share(_path: &Path) {}

/// The file extension the content says it has, for a saved file whose name lacks one (stickers arrive named «Image»).
pub fn extension_of(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

/// Gives a saved extensionless file the extension of its content; returns the path to share.
pub fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() { return path.to_owned(); }
    let mut head = [0u8; 12];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let Some(ext) = extension_of(&head[..n]) else { return path.to_owned() };
    let renamed = path.with_extension(ext);
    if std::fs::rename(path, &renamed).is_ok() { renamed } else { path.to_owned() }
}

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
    fn extension_follows_the_content() {
        assert_eq!(extension_of(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(extension_of(&[0x89, b'P', b'N', b'G', 13, 10]), Some("png"));
        assert_eq!(extension_of(b"GIF89a"), Some("gif"));
        assert_eq!(extension_of(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(extension_of(b"hello"), None);
        let dir = std::env::temp_dir().join(format!("gnobbi-ext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bare = dir.join("Image");
        std::fs::write(&bare, b"GIF89a....").unwrap();
        assert_eq!(with_extension(&bare), dir.join("Image.gif"));
        assert!(!bare.exists());
        assert_eq!(with_extension(&dir.join("a.png")), dir.join("a.png"));
        let _ = std::fs::remove_dir_all(dir);
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
