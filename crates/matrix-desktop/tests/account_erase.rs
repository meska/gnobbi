use matrix_desktop::{AccountCatalog, AppDirs, erase_account};
use matrix_domain::AccountId;
use std::{cell::RefCell, path::PathBuf};

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("gnobbi-erase-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

/// Catalog of `n` accounts, each with a store folder and a cache file on disk.
fn setup(d: &AppDirs, n: usize) -> Vec<AccountId> {
    let mut c = AccountCatalog::default();
    let ids: Vec<AccountId> = (0..n)
        .map(|i| c.add(&format!("@u{i}:example.org"), "https://example.org", ""))
        .collect();
    c.save(d).unwrap();
    for id in &ids {
        c.ensure_dirs(d, *id).unwrap();
        std::fs::create_dir_all(c.data_dir(d, *id).unwrap().join("account-1")).unwrap();
        std::fs::write(
            c.data_dir(d, *id).unwrap().join("account-1/session.sealed"),
            b"x",
        )
        .unwrap();
        std::fs::write(c.cache_dir(d, *id).unwrap().join("thumb"), b"x").unwrap();
    }
    ids
}

/// Every file below `root` (the shared empty parents are not account residue).
fn files_under(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

#[test]
fn removing_one_of_several_leaves_the_others_untouched() {
    let d = dirs("one");
    let ids = setup(&d, 3);
    let before = AccountCatalog::load(&d);
    let (keep_a, keep_b) = (
        before.data_dir(&d, ids[0]).unwrap(),
        before.cache_dir(&d, ids[2]).unwrap(),
    );
    let seen = RefCell::new(vec![]);
    erase_account(&d, ids[1], |stores| {
        seen.replace(stores.to_vec());
        true
    })
    .unwrap();
    let after = AccountCatalog::load(&d);
    assert_eq!(after.accounts().len(), 2);
    assert!(after.get(ids[1]).is_none());
    assert!(!before.data_dir(&d, ids[1]).unwrap().exists());
    assert!(!before.cache_dir(&d, ids[1]).unwrap().exists());
    // the vault is asked about this account's store folders only
    assert_eq!(
        *seen.borrow(),
        vec![before.data_dir(&d, ids[1]).unwrap().join("account-1")]
    );
    assert!(keep_a.join("account-1/session.sealed").exists() && keep_b.join("thumb").exists());
    // an ID is never reused
    assert!(AccountCatalog::load(&d).next_root(&d).ends_with("4"));
}

#[test]
fn removing_the_last_account_leaves_no_trace_on_disk() {
    let d = dirs("last");
    let ids = setup(&d, 1);
    erase_account(&d, ids[0], |_| true).unwrap();
    assert!(AccountCatalog::load(&d).accounts().is_empty());
    assert_eq!(files_under(&d.data), Vec::<PathBuf>::new());
    assert_eq!(files_under(&d.cache), Vec::<PathBuf>::new());
}

#[test]
fn removing_the_shown_account_of_two_keeps_the_other_one_complete() {
    let d = dirs("shown");
    let ids = setup(&d, 2);
    let other_files = files_under(&AccountCatalog::load(&d).data_dir(&d, ids[1]).unwrap());
    erase_account(&d, ids[0], |_| true).unwrap();
    let c = AccountCatalog::load(&d);
    assert_eq!(c.accounts().len(), 1);
    assert_eq!(files_under(&c.data_dir(&d, ids[1]).unwrap()), other_files);
}

#[test]
fn a_vault_failure_leaves_disk_and_catalog_as_they_were() {
    let d = dirs("vault-fails");
    let ids = setup(&d, 2);
    let before = AccountCatalog::load(&d);
    assert!(erase_account(&d, ids[0], |_| false).is_err());
    assert_eq!(AccountCatalog::load(&d), before);
    assert!(
        before
            .data_dir(&d, ids[0])
            .unwrap()
            .join("account-1/session.sealed")
            .exists()
    );
    assert!(erase_account(&d, AccountId::new(99), |_| true).is_err());
}

#[test]
fn a_legacy_profile_folder_is_removed_with_all_its_stores() {
    let d = dirs("legacy");
    std::fs::create_dir_all(d.data.join("profile/account-1")).unwrap();
    std::fs::create_dir_all(d.data.join("profile/account-2")).unwrap();
    std::fs::write(d.data.join("profile/account-2/session.sealed"), b"x").unwrap();
    let mut c = AccountCatalog::default();
    assert!(c.migrate_legacy(&d));
    c.save(&d).unwrap();
    let id = c.accounts()[0].id;
    let seen = RefCell::new(0);
    erase_account(&d, id, |stores| {
        seen.replace(stores.len());
        true
    })
    .unwrap();
    assert_eq!(*seen.borrow(), 2);
    assert!(!d.data.join("profile").exists());
}
