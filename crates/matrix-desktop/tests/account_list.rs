use matrix_desktop::{AccountCatalog, AppDirs, account_rows};

fn dirs() -> AppDirs {
    let root = std::env::temp_dir().join(format!("gnobbi-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

#[test]
fn one_two_three_accounts_in_catalog_order_with_current_marked() {
    let mut c = AccountCatalog::default();
    assert!(account_rows(&c, None).is_empty());
    c.add("@ada:x.org", "https://x.org", "Ada");
    let rows = account_rows(&c, Some("@ada:x.org"));
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].name.as_str(),
            rows[0].initials.as_str(),
            rows[0].current
        ),
        ("Ada", "Ad", true)
    );
    c.add("@bob:x.org", "https://x.org", "");
    c.add("@cy:y.org", "https://y.org", "Cy");
    let rows = account_rows(&c, Some("@bob:x.org"));
    let seen: Vec<_> = rows
        .iter()
        .map(|r| (r.name.as_str(), r.user_id.as_str(), r.current))
        .collect();
    assert_eq!(
        seen,
        [
            ("Ada", "@ada:x.org", false),
            ("@bob:x.org", "@bob:x.org", true),
            ("Cy", "@cy:y.org", false)
        ]
    );
    assert_eq!(rows[1].initials, "bo");
}

#[test]
fn login_on_next_root_registers_without_touching_others() {
    let d = dirs();
    let mut c = AccountCatalog::default();
    let first = c.add("@ada:x.org", "https://x.org", "Ada");
    let root = c.next_root(&d);
    assert!(c.record_login(&d, &root, "@bob:x.org", "Bob"));
    assert_eq!(c.accounts().len(), 2);
    assert_eq!(c.get(first).unwrap().user_id.as_deref(), Some("@ada:x.org"));
    // repeat, foreign root and duplicate user are no-ops
    let bob_root = c.data_dir(&d, c.accounts()[1].id).unwrap();
    assert!(!c.record_login(&d, &bob_root, "@bob:x.org", "Bob"));
    assert!(!c.record_login(&d, &d.data.join("elsewhere"), "@eve:x.org", "Eve"));
    let next = c.next_root(&d);
    assert!(!c.record_login(&d, &next, "@ada:x.org", "Ada"));
    assert_eq!(c.accounts().len(), 2);
}

#[test]
fn login_fills_the_adopted_legacy_entry_in_place() {
    let d = dirs();
    std::fs::create_dir_all(d.data.join("profile/account-1")).unwrap();
    std::fs::write(d.data.join("profile/account-1/s"), b"x").unwrap();
    let mut c = AccountCatalog::default();
    assert!(c.migrate_legacy(&d));
    assert!(account_rows(&c, None).is_empty());
    assert!(c.record_login(&d, &d.data.join("profile"), "@ada:x.org", "Ada"));
    assert_eq!(c.accounts().len(), 1);
    assert_eq!(account_rows(&c, Some("@ada:x.org"))[0].name, "Ada");
}

#[test]
fn first_login_on_a_fresh_install_is_listed() {
    let d = dirs();
    let mut c = AccountCatalog::default();
    assert!(c.record_login(&d, &d.data.join("profile"), "@ada:x.org", "Ada"));
    assert_eq!(
        c.data_dir(&d, c.accounts()[0].id),
        Some(d.data.join("profile"))
    );
    let next = c.next_root(&d);
    assert!(c.record_login(&d, &next, "@bob:x.org", "Bob"));
    assert_eq!(c.accounts().len(), 2);
}
