//! Accessible tree of the drawer with two accounts: who is marked «account attivo», where the tree puts focus,
//! and that the unread count is announced once. Visual side (focus ring, switch) lives in src/drawer_active_test.rs.
#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use matrix_desktop::accessibility::drawer::*;
use matrix_domain::AccountId;

fn row(id: u64, name: &str, unread: u64, current: bool) -> DrawerRow {
    DrawerRow { id: AccountId::new(id), name: name.into(), user_id: format!("@{name}:s.org"), unread, current, switchable: !current }
}
fn snap(rows: Vec<DrawerRow>) -> DrawerSnapshot {
    DrawerSnapshot { rows, confirm: None, connected: true }
}
fn labels(s: &DrawerSnapshot) -> Vec<String> {
    s.tree().nodes.iter().filter_map(|(_, n)| n.label().map(str::to_string)).collect()
}
fn count(s: &DrawerSnapshot, part: &str) -> usize {
    labels(s).iter().filter(|l| l.contains(part)).count()
}

#[test]
fn only_the_shown_account_is_marked_active_whatever_its_position() {
    for active_first in [true, false] {
        let s = snap(vec![row(1, "anna", 0, active_first), row(2, "bob", 0, !active_first)]);
        let (on, off) = if active_first { ("anna", "bob") } else { ("bob", "anna") };
        assert_eq!(count(&s, "account attivo"), 1, "{:?}", labels(&s));
        assert!(labels(&s).iter().any(|l| l.starts_with(on) && l.ends_with(", account attivo")));
        assert!(labels(&s).iter().any(|l| l.starts_with(off) && !l.contains("attivo")));
    }
}

#[test]
fn single_account_is_active_and_the_tree_is_still_complete() {
    let s = snap(vec![row(1, "anna", 0, true)]);
    assert_eq!(count(&s, "account attivo"), 1);
    assert_eq!(labels(&s).iter().filter(|l| l.starts_with("Rimuovi account")).count(), 1);
}

#[test]
fn unread_is_announced_once_per_row_also_on_the_active_row() {
    let s = snap(vec![row(1, "anna", 4, true), row(2, "bob", 1, false)]);
    let l = labels(&s);
    assert_eq!(l.iter().filter(|l| l.contains("non lett")).count(), 2, "{l:?}");
    assert!(l.contains(&"anna, @anna:s.org, 4 messaggi non letti, account attivo".to_string()));
    assert!(l.contains(&"bob, @bob:s.org, 1 messaggio non letto".to_string()));
    for x in &l {
        assert!(x.matches("non lett").count() <= 1 && x.matches("attivo").count() <= 1, "duplicated: {x}");
    }
}

/// Initial accessible focus must be the shown account's row, as the visual focus is (05ca360).
#[test]
#[ignore = "difetto aperto: il fuoco dell'albero va sul primo nodo («Chiudi menu»), non sulla riga attiva"]
fn tree_focus_starts_on_the_active_row() {
    for rows in [
        vec![row(1, "anna", 0, true), row(2, "bob", 0, false)],
        vec![row(1, "anna", 0, false), row(2, "bob", 0, true)],
        vec![row(1, "anna", 0, true)],
    ] {
        let s = snap(rows);
        let t = s.tree();
        let active = t.nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.ends_with("account attivo"))).unwrap().0;
        assert_eq!(t.focus, active, "focus on {:?}", t.nodes.iter().find(|(i, _)| *i == t.focus).and_then(|(_, n)| n.label()));
    }
}
