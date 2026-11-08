#![cfg(target_os = "linux")]
//! «Aggiungi al menu» copies a downloaded AppImage to ~/Applications, under a temporary HOME.
use matrix_desktop::IconChoice;
use matrix_desktop::desktop_entry::{
    Outcome, Placed,
    detect::{EntryStatus, Env, entry_status},
    install_resolved, place_appimage,
};
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// `root/{home,Downloads/g.AppImage}`; returns (home, download, share).
fn sandbox(name: &str, bytes: &[u8]) -> (PathBuf, PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("appimage-copy-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("home")).unwrap();
    fs::create_dir_all(root.join("Downloads")).unwrap();
    let dl = root.join("Downloads/g.AppImage");
    fs::write(&dl, bytes).unwrap();
    (root.join("home"), dl, root.join("home/.local/share"))
}

fn install(
    home: &Path,
    share: &Path,
    app: Option<&Path>,
    replace: bool,
) -> Result<Outcome, String> {
    install_resolved(
        home,
        share,
        app,
        Some(Path::new("/usr/bin/gnobbi")),
        IconChoice::default(),
        replace,
        &mut |_, _| {},
    )
}

fn desktop(share: &Path) -> String {
    fs::read_to_string(share.join("applications/net.meskatech.gnobbi.desktop")).unwrap_or_default()
}

fn leftovers(home: &Path) -> Vec<String> {
    fs::read_dir(home.join("Applications"))
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn copy_succeeds_executable_same_bytes_exec_on_copy_original_kept() {
    let (home, dl, share) = sandbox("ok", b"appimage-v1");
    assert_eq!(install(&home, &share, Some(&dl), false), Ok(Outcome::Done));
    let copy = home.join("Applications/gnobbi.AppImage");
    assert_eq!(fs::read(&copy).unwrap(), b"appimage-v1");
    assert_ne!(fs::metadata(&copy).unwrap().permissions().mode() & 0o111, 0);
    assert!(desktop(&share).contains(&format!("Exec=\"{}\"", copy.display())));
    assert!(dl.exists());
    assert_eq!(leftovers(&home), ["gnobbi.AppImage"]);
}

#[test]
fn identical_copy_is_not_rewritten() {
    let (home, dl, share) = sandbox("same", b"v1");
    install(&home, &share, Some(&dl), false).unwrap();
    let copy = home.join("Applications/gnobbi.AppImage");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&copy)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert_eq!(install(&home, &share, Some(&dl), false), Ok(Outcome::Done));
    assert_eq!(fs::metadata(&copy).unwrap().modified().unwrap(), old);
    assert!(desktop(&share).contains("Applications/gnobbi.AppImage"));
}

#[test]
fn different_copy_needs_confirmation_then_is_replaced() {
    let (home, dl, share) = sandbox("diff", b"v1");
    install(&home, &share, Some(&dl), false).unwrap();
    fs::write(&dl, b"v2-longer").unwrap();
    let copy = home.join("Applications/gnobbi.AppImage");
    let before = desktop(&share);
    assert_eq!(
        install(&home, &share, Some(&dl), false),
        Ok(Outcome::NeedsConfirm)
    );
    assert_eq!(
        (fs::read(&copy).unwrap(), desktop(&share)),
        (b"v1".to_vec(), before)
    );
    assert_eq!(install(&home, &share, Some(&dl), true), Ok(Outcome::Done));
    assert_eq!(fs::read(&copy).unwrap(), b"v2-longer");
    assert_eq!(leftovers(&home), ["gnobbi.AppImage"]);
    assert!(dl.exists());
}

struct Full;
impl io::Read for Full {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::StorageFull.into())
    }
}

#[test]
fn disk_full_is_one_line_and_leaves_no_temp_and_no_desktop() {
    let (home, dl, share) = sandbox("full", b"v1");
    let err = place_appimage(&home, &dl, false, &|| Ok(Box::new(Full))).unwrap_err();
    assert_eq!(err, "Spazio su disco insufficiente");
    assert!(leftovers(&home).is_empty(), "{:?}", leftovers(&home));
    assert!(desktop(&share).is_empty());
}

#[test]
fn other_io_errors_are_one_line() {
    let (home, _, _) = sandbox("io", b"v1");
    let err = place_appimage(
        &home,
        &home.join("missing.AppImage"),
        false,
        &|| unreachable!(),
    )
    .unwrap_err();
    assert!(!err.is_empty() && !err.contains('\n'));
}

#[test]
fn no_appimage_or_already_in_applications_means_no_copy() {
    let (home, dl, share) = sandbox("nocopy", b"v1");
    // no $APPIMAGE: Exec is current_exe, nothing under ~/Applications
    assert_eq!(install(&home, &share, None, false), Ok(Outcome::Done));
    assert!(desktop(&share).contains("/usr/bin/gnobbi"));
    assert!(!home.join("Applications").exists());
    // already inside ~/Applications (even via a symlinked path): used as is
    fs::create_dir_all(home.join("Applications/sub")).unwrap();
    let inside = home.join("Applications/sub/mine.AppImage");
    fs::copy(&dl, &inside).unwrap();
    assert_eq!(
        place_appimage(&home, &inside, false, &|| unreachable!()),
        Ok(Placed::Elsewhere)
    );
    install(&home, &share, Some(&inside), false).unwrap();
    assert!(desktop(&share).contains("sub/mine.AppImage"));
    assert_eq!(leftovers(&home), ["sub"]);
}

#[test]
fn detect_is_ok_for_download_with_identical_copy_and_stale_when_it_differs() {
    let (home, dl, share) = sandbox("detect", b"v1");
    install(&home, &share, Some(&dl), false).unwrap();
    let env = Env {
        home: Some(home.clone()),
        xdg_data_dirs: Some(home.join("sys").to_string_lossy().into_owned()),
        exec: Some(dl.to_string_lossy().into_owned()),
        graphical: true,
        ..Env::default()
    };
    assert_eq!(entry_status(&env), EntryStatus::Ok);
    fs::write(&dl, b"v2").unwrap();
    assert_eq!(entry_status(&env), EntryStatus::Stale);
}
