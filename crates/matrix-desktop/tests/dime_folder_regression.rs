//! Independent QA of startup with odd `Dime` folders next to a temporary HOME (never real data).
#![cfg(unix)]
use matrix_desktop::{AccountCatalog, Platform, app_dirs, default_signin_args};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("gnobbi-qa-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).unwrap();
    h
}

/// Launch sequence on a fresh HOME; returns whether it behaved as a first launch.
fn first_launch(platform: Platform, home: &Path) {
    let h = home.to_str().unwrap().to_owned();
    let dirs = app_dirs(platform, |k| (k == "HOME").then(|| h.clone())).unwrap();
    dirs.ensure().expect("ensure must not fail");
    let args = default_signin_args(&["--signin".to_owned()], &dirs);
    assert_eq!(args, ["--signin", dirs.data.join("profile").to_str().unwrap()]);
    let mut c = AccountCatalog::load(&dirs);
    assert!(!c.migrate_legacy(&dirs));
    assert!(c.accounts().is_empty() && !c.is_damaged());
    assert!(!dirs.data.join("profile").exists());
}

const OLD: [&str; 3] = ["Library/Application Support/Dime", ".local/share/dime", "Dime"];

#[test]
fn no_dime_folder_is_plain_first_launch() {
    for p in [Platform::MacOs, Platform::Linux] {
        let home = tmp("none");
        first_launch(p, &home);
        for o in OLD {
            assert!(!home.join(o).exists(), "{o} must not be created");
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn empty_dime_folders_no_error_and_not_filled() {
    for p in [Platform::MacOs, Platform::Linux] {
        let home = tmp("empty");
        for o in OLD {
            std::fs::create_dir_all(home.join(o)).unwrap();
        }
        first_launch(p, &home);
        for o in OLD {
            assert_eq!(std::fs::read_dir(home.join(o)).unwrap().count(), 0, "{o}");
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn readonly_and_unreadable_dime_folders_no_error() {
    for mode in [0o500, 0o000] {
        for p in [Platform::MacOs, Platform::Linux] {
            let home = tmp("ro");
            for o in OLD {
                let prof = home.join(o).join("profile/account-1");
                std::fs::create_dir_all(&prof).unwrap();
                std::fs::write(prof.join("session.sealed"), b"old").unwrap();
            }
            for o in OLD {
                std::fs::set_permissions(home.join(o), std::fs::Permissions::from_mode(mode)).unwrap();
            }
            first_launch(p, &home);
            for o in OLD {
                std::fs::set_permissions(home.join(o), std::fs::Permissions::from_mode(0o700)).unwrap();
                assert_eq!(std::fs::read(home.join(o).join("profile/account-1/session.sealed")).unwrap(), b"old");
            }
            std::fs::remove_dir_all(home).unwrap();
        }
    }
}
