use matrix_desktop::legacy_profile::{adopt, legacy_path, offered, renamed_path};
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("dime-legacy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn legacy_with_file(root: &Path) -> PathBuf {
    let legacy = root.join("home/Dime/profile");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("store.db"), b"x").unwrap();
    legacy
}

#[test]
fn legacy_path_is_dime_profile_under_home() {
    assert_eq!(
        legacy_path(Path::new("/h")),
        PathBuf::from("/h/Dime/profile")
    );
}

#[test]
fn offered_only_when_legacy_exists_and_new_profile_is_absent() {
    let root = tmp("offered");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    assert!(!offered(&root.join("home/Dime/profile"), &data));
    let legacy = legacy_with_file(&root);
    assert!(offered(&legacy, &data));
    std::fs::create_dir_all(data.join("profile")).unwrap();
    std::fs::write(data.join("profile/x"), b"x").unwrap();
    assert!(!offered(&legacy, &data));
}

#[test]
fn adopt_moves_profile_and_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = tmp("adopt");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let legacy = legacy_with_file(&root);
    std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o700)).unwrap();
    adopt(&legacy, &data).unwrap();
    assert!(!legacy.exists());
    assert_eq!(std::fs::read(data.join("profile/store.db")).unwrap(), b"x");
    let mode = std::fs::metadata(data.join("profile"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn ignoring_leaves_everything_in_place() {
    let root = tmp("ignore");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let legacy = legacy_with_file(&root);
    assert!(offered(&legacy, &data));
    assert!(legacy.join("store.db").exists());
    assert!(!data.join("profile").exists());
}

#[test]
fn adopt_refuses_to_overwrite_an_existing_profile() {
    let root = tmp("overwrite");
    let data = root.join("data");
    std::fs::create_dir_all(data.join("profile")).unwrap();
    std::fs::write(data.join("profile/x"), b"x").unwrap();
    let legacy = legacy_with_file(&root);
    assert!(adopt(&legacy, &data).is_err());
    assert!(legacy.join("store.db").exists());
}

#[test]
fn failed_rename_across_devices_leaves_legacy_untouched() {
    use std::os::unix::fs::MetadataExt;
    let root = tmp("xdev");
    let legacy = legacy_with_file(&root);
    let shm = Path::new("/dev/shm");
    if !shm.is_dir()
        || std::fs::metadata(shm).unwrap().dev() == std::fs::metadata(&root).unwrap().dev()
    {
        eprintln!("skipped: no second filesystem available");
        return;
    }
    let data = shm.join(format!("dime-legacy-xdev-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let result = adopt(&legacy, &data);
    let _ = std::fs::remove_dir_all(&data);
    assert!(result.is_err(), "cross-device move must fail, not copy");
    assert!(legacy.join("store.db").exists());
}

#[test]
fn empty_profile_dir_made_at_startup_does_not_block_the_offer_or_adoption() {
    let root = tmp("emptydir");
    let data = root.join("data");
    std::fs::create_dir_all(data.join("profile")).unwrap();
    let legacy = legacy_with_file(&root);
    assert!(offered(&legacy, &data));
    adopt(&legacy, &data).unwrap();
    assert_eq!(std::fs::read(data.join("profile/store.db")).unwrap(), b"x");
}

#[test]
fn non_empty_new_profile_is_never_replaced() {
    let root = tmp("nonempty");
    let data = root.join("data");
    std::fs::create_dir_all(data.join("profile")).unwrap();
    std::fs::write(data.join("profile/keep"), b"k").unwrap();
    let legacy = legacy_with_file(&root);
    assert!(!offered(&legacy, &data));
    assert!(adopt(&legacy, &data).is_err());
    assert!(legacy.join("store.db").exists() && data.join("profile/keep").exists());
}

#[test]
fn renamed_path_is_the_old_name_profile_next_to_the_new_data_dir() {
    assert_eq!(
        renamed_path(Path::new("/h/.local/share/gnobbi")),
        Some(PathBuf::from("/h/.local/share/dime/profile"))
    );
    assert_eq!(renamed_path(Path::new("/")), None);
}

#[test]
fn old_name_profile_is_offered_and_adopted_without_touching_other_files() {
    let root = tmp("renamed");
    let data = root.join("share/gnobbi");
    std::fs::create_dir_all(&data).unwrap();
    let old = renamed_path(&data).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("store.db"), b"x").unwrap();
    assert!(offered(&old, &data));
    adopt(&old, &data).unwrap();
    assert_eq!(std::fs::read(data.join("profile/store.db")).unwrap(), b"x");
    assert!(!old.exists());
}
