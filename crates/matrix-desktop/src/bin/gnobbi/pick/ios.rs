//! iOS pickers with a multiple selection: PHPicker (`selectionLimit` 0) for pictures and videos, UIDocumentPicker
//! (`allowsMultipleSelection`) for any file. robius-file-picker only does one file per pass. Every picked file is copied
//! into its own `robius-file-picker-*` temp folder (what `purge_temp` clears) under its own name.
use block2::RcBlock;
use objc2::{
    define_class, extern_class, extern_conformance, extern_methods, extern_protocol, msg_send,
    rc::{Allocated, Retained},
    runtime::ProtocolObject,
    AnyThread, DeclaredClass, MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSURL};
use objc2_photos_ui::{PHPickerConfiguration, PHPickerConfigurationAssetRepresentationMode, PHPickerFilter, PHPickerResult};
use objc2_ui_kit::{
    UIAdaptivePresentationControllerDelegate, UIApplication, UIDocumentPickerDelegate, UIDocumentPickerViewController,
    UIPresentationController, UIResponder, UIViewController,
};
use objc2_uniform_type_identifiers::{UTTypeImage, UTTypeItem, UTTypeMovie};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

type Done = Box<dyn FnOnce(Vec<PathBuf>) + Send>;

// The delegate protocol and the view controller are not in objc2-photos-ui (as in robius-file-picker).
extern_protocol!(
    unsafe trait PHPickerViewControllerDelegate: NSObjectProtocol + MainThreadOnly {
        #[unsafe(method(picker:didFinishPicking:))]
        #[unsafe(method_family = none)]
        unsafe fn picker_did_finish_picking(&self, picker: &PHPickerViewController, results: &NSArray<PHPickerResult>);
    }
);

extern_class!(
    #[unsafe(super(UIViewController, UIResponder, NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct PHPickerViewController;
);

extern_conformance!(
    unsafe impl NSObjectProtocol for PHPickerViewController {}
);

impl PHPickerViewController {
    extern_methods!(
        #[unsafe(method(setDelegate:))]
        #[unsafe(method_family = none)]
        unsafe fn set_delegate(&self, delegate: Option<&ProtocolObject<dyn PHPickerViewControllerDelegate>>);

        #[unsafe(method(initWithConfiguration:))]
        #[unsafe(method_family = init)]
        unsafe fn init_with_configuration(this: Allocated<Self>, configuration: &PHPickerConfiguration) -> Retained<Self>;
    );
}

struct Ivars {
    done: Cell<Option<Done>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl PHPickerViewControllerDelegate for Delegate {
        #[unsafe(method(picker:didFinishPicking:))]
        unsafe fn picker_did_finish_picking(&self, picker: &PHPickerViewController, results: &NSArray<PHPickerResult>) {
            picker.dismissViewControllerAnimated_completion(true, None);
            let Some(done) = self.ivars().done.take() else { return };
            load_all(results, done);
        }
    }

    unsafe impl UIDocumentPickerDelegate for Delegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        #[allow(non_snake_case)]
        unsafe fn documentPicker_didPickDocumentsAtURLs(&self, _: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            if let Some(done) = self.ivars().done.take() {
                done(urls.iter().filter_map(|url| copy_in(&url, true)).collect());
            }
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        #[allow(non_snake_case)]
        unsafe fn documentPickerWasCancelled(&self, _: &UIDocumentPickerViewController) {
            self.cancel();
        }
    }

    unsafe impl UIAdaptivePresentationControllerDelegate for Delegate {
        #[unsafe(method(presentationControllerDidDismiss:))]
        #[allow(non_snake_case)]
        unsafe fn presentationControllerDidDismiss(&self, _: &UIPresentationController) {
            self.cancel();
        }
    }
);

impl Delegate {
    fn cancel(&self) {
        if let Some(done) = self.ivars().done.take() {
            done(Vec::new());
        }
    }
}

thread_local! {
    /// The pickers keep their delegate weakly: this holds the one of the open dialog (main thread only).
    static LIVE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

/// Opens the picker (`media`: PHPicker, else documents) and calls `done` once with the copies of the chosen files in
/// the order they were chosen; empty when the dialog was closed, could not open or nothing could be read.
pub fn pick_many(media: bool, done: impl FnOnce(Vec<PathBuf>) + Send + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return done(Vec::new()) };
    #[allow(deprecated)] // as in pick::share
    let Some(root) = UIApplication::sharedApplication(mtm).keyWindow().and_then(|w| w.rootViewController()) else { return done(Vec::new()) };
    let mut top = root;
    while let Some(next) = top.presentedViewController() { top = next; }
    let delegate = Delegate::alloc(mtm).set_ivars(Ivars { done: Cell::new(Some(Box::new(done))) });
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    let picker: Retained<UIViewController> = if media {
        let config = unsafe { PHPickerConfiguration::init(PHPickerConfiguration::alloc()) };
        let filter = unsafe { PHPickerFilter::anyFilterMatchingSubfilters(&NSArray::from_retained_slice(&[PHPickerFilter::imagesFilter(), PHPickerFilter::videosFilter()])) };
        unsafe {
            config.setSelectionLimit(0); // 0: no limit; the preview takes up to 10 and says so
            config.setFilter(Some(&filter));
            // A HEIC photo comes back as JPEG, which every receiving client can show.
            config.setPreferredAssetRepresentationMode(PHPickerConfigurationAssetRepresentationMode::Compatible);
        }
        let picker = unsafe { PHPickerViewController::init_with_configuration(PHPickerViewController::alloc(mtm), &config) };
        unsafe { picker.set_delegate(Some(ProtocolObject::from_ref(&*delegate))) };
        Retained::into_super(picker)
    } else {
        let types = NSArray::from_slice(&[unsafe { UTTypeItem }]);
        let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(UIDocumentPickerViewController::alloc(mtm), &types, true);
        picker.setAllowsMultipleSelection(true);
        picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Retained::into_super(picker)
    };
    LIVE.with(|l| *l.borrow_mut() = Some(delegate.clone()));
    top.presentViewController_animated_completion(&picker, true, None);
    // The sheet's swipe-down does not tell the picker's delegate.
    if let Some(presentation) = picker.presentationController() {
        unsafe { presentation.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    }
}

/// What the loads of one PHPicker answer have in common: the copies by position, how many are still coming, who to tell.
struct Gather {
    files: Mutex<Vec<Option<PathBuf>>>,
    left: AtomicUsize,
    done: Mutex<Option<Done>>,
}

impl Gather {
    fn put(&self, i: usize, file: Option<PathBuf>) {
        self.files.lock().unwrap_or_else(|e| e.into_inner())[i] = file;
        if self.left.fetch_sub(1, Ordering::AcqRel) == 1 {
            let files = std::mem::take(&mut *self.files.lock().unwrap_or_else(|e| e.into_inner()));
            if let Some(done) = self.done.lock().unwrap_or_else(|e| e.into_inner()).take() {
                done(files.into_iter().flatten().collect());
            }
        }
    }
}

/// Loads every chosen item to a temp file (the system deletes its own file when the handler returns).
fn load_all(results: &NSArray<PHPickerResult>, done: Done) {
    if results.is_empty() {
        return done(Vec::new());
    }
    let gather = Arc::new(Gather { files: Mutex::new(vec![None; results.len()]), left: AtomicUsize::new(results.len()), done: Mutex::new(Some(done)) });
    for (i, result) in results.iter().enumerate() {
        let provider = unsafe { result.itemProvider() };
        // A Live Photo comes as its still picture.
        let kind = unsafe { if provider.hasItemConformingToTypeIdentifier(&UTTypeImage.identifier()) { UTTypeImage } else { UTTypeMovie } };
        let (g, keep) = (gather.clone(), provider.clone());
        let block = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
            let _ = &keep;
            // SAFETY: a non-null url is a valid NSURL for the duration of this handler.
            let file = if error.is_null() && !url.is_null() { copy_in(unsafe { &*url }, false) } else { None };
            g.put(i, file);
        });
        unsafe { provider.loadFileRepresentationForTypeIdentifier_completionHandler(&kind.identifier(), &block) };
    }
}

/// Copies (`consume`: moves) the file at `url` into a fresh temp folder, keeping its name.
fn copy_in(url: &NSURL, consume: bool) -> Option<PathBuf> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let from = PathBuf::from(url.path()?.to_string());
    let dir = std::env::temp_dir().join(format!("robius-file-picker-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).ok()?;
    let to = dir.join(crate::pick::safe_name(&from.file_name()?.to_string_lossy()));
    std::fs::copy(&from, &to).ok()?;
    if consume {
        let _ = std::fs::remove_file(&from);
    }
    Some(to)
}
