//! The old `Dime` folders are never read, offered, migrated or written: startup treats them as absent.
#![cfg(unix)] // POSIX home layout; Windows has no old-name folder to fake here
use matrix_desktop::{AccountCatalog, AppDirs, Platform, app_dirs, default_signin_args};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

type Snapshot = Vec<(PathBuf, Option<Vec<u8>>, SystemTime)>;

/// Every path under `root` with content (files) and mtime; any write, touch or removal changes it.
fn snapshot(root: &Path) -> Snapshot {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(path) = todo.pop() {
        let meta = std::fs::symlink_metadata(&path).unwrap();
        let bytes = meta.is_file().then(|| std::fs::read(&path).unwrap());
        out.push((path.clone(), bytes, meta.modified().unwrap()));
        if meta.is_dir() {
            todo.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        }
    }
    out.sort();
    out
}

/// Home with the old folders of every platform; the sealed session sits in `Dime/profile`.
fn home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("gnobbi-no-dime-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    for old in ["Library/Application Support/Dime/profile", ".local/share/dime/profile", "Dime/profile"] {
        let profile = home.join(old);
        std::fs::create_dir_all(profile.join("account-1")).unwrap();
        std::fs::write(profile.join("account-1/session.sealed"), b"old").unwrap();
    }
    home
}

/// What the app does at launch before any sign-in: directories, default profile, catalog.
fn start(platform: Platform, home: &Path) -> (AppDirs, Vec<String>, AccountCatalog) {
    let home = home.to_str().unwrap().to_owned();
    let dirs = app_dirs(platform, |k| (k == "HOME").then(|| home.clone())).unwrap();
    dirs.ensure().unwrap();
    let args = default_signin_args(&["--signin".to_owned()], &dirs);
    let mut catalog = AccountCatalog::load(&dirs);
    assert!(!catalog.migrate_legacy(&dirs), "nothing to adopt");
    (dirs, args, catalog)
}

fn old_folders(home: &Path) -> Vec<PathBuf> {
    ["Library/Application Support/Dime", ".local/share/dime", "Dime"].map(|p| home.join(p)).to_vec()
}

#[test]
fn only_old_dime_folders_behave_like_first_launch_and_stay_untouched() {
    for (name, platform) in [("mac", Platform::MacOs), ("linux", Platform::Linux)] {
        let home = home(name);
        let before: Vec<_> = old_folders(&home).iter().map(|p| snapshot(p)).collect();
        let (dirs, args, catalog) = start(platform, &home);
        assert!(catalog.accounts().is_empty() && !catalog.is_damaged());
        assert_eq!(args, ["--signin", dirs.data.join("profile").to_str().unwrap()]);
        assert!(!dirs.data.join("profile").exists(), "no profile adopted from the old folder");
        let after: Vec<_> = old_folders(&home).iter().map(|p| snapshot(p)).collect();
        assert_eq!(before, after, "{name}: old folders changed");
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn gnobbi_catalog_next_to_old_dime_folder_is_unchanged() {
    let home = home("catalog");
    let home_str = home.to_str().unwrap().to_owned();
    let dirs = app_dirs(Platform::Linux, |k| (k == "HOME").then(|| home_str.clone())).unwrap();
    dirs.ensure().unwrap();
    let mut seeded = AccountCatalog::load(&dirs);
    let id = seeded.add("@a:example.org", "https://example.org", "Casa");
    seeded.save(&dirs).unwrap();
    seeded.ensure_dirs(&dirs, id).unwrap();
    let gnobbi = snapshot(&dirs.data);
    let old: Vec<_> = old_folders(&home).iter().map(|p| snapshot(p)).collect();

    let (_, _, catalog) = start(Platform::Linux, &home);

    assert_eq!(catalog, seeded);
    assert_eq!(snapshot(&dirs.data), gnobbi);
    assert_eq!(old_folders(&home).iter().map(|p| snapshot(p)).collect::<Vec<_>>(), old);
    std::fs::remove_dir_all(home).unwrap();
}
