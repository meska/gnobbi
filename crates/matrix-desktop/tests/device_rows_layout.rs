//! Device panel: row layout (source-level, as room_list_clutter.rs) and the accessible tree.
//! shortcut: layout is checked on the view source, upgrade to a rendered-tree assert when one exists.
#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, Role};
use matrix_desktop::DeviceVerification;
use matrix_desktop::accessibility::devices::{DeviceControl, DeviceNodes, DeviceOverview};
use matrix_domain::AccountId;
use matrix_engine::verification::{VerificationDevice, VerificationEvent};

const VIEW: &str = include_str!("../src/verification_view.rs");
const A11Y: &str = include_str!("../src/accessibility/devices.rs");
const ALL_SOURCES: [(&str, &str); 4] = [
    ("verification_view.rs", VIEW),
    ("accessibility/devices.rs", A11Y),
    (
        "tests/device_accessibility.rs",
        include_str!("device_accessibility.rs"),
    ),
    ("tests/verification.rs", include_str!("verification.rs")),
];
const REMOVED: [&str; 4] = [
    "Avvia solo una verifica attesa",
    "canale fidato",
    "non confermare se sono diversi",
    "del tuo account · SAS",
];

/// The device row: from its `.id(("verification-device", ..))` to the click handler.
fn row_source() -> &'static str {
    let start = VIEW.find("(\"verification-device\", index)").expect("row div");
    let end = start + VIEW[start..].find(".on_click").expect("row click");
    &VIEW[start..end]
}

#[test]
fn row_height_follows_content_and_never_clips() {
    let row = row_source();
    assert!(!row.contains(".h(px("), "fixed row height");
    assert!(!row.contains(".h_"), "fixed row height");
    assert!(row.contains(".min_h(px("), "row needs a minimum, not a fixed, height");
    assert!(row.contains(".flex_shrink_0()"), "row must not shrink inside the scroll area");
    for clip in [".overflow_hidden", ".truncate()", ".whitespace_nowrap", ".text_ellipsis"] {
        // Only the ID line (a child of the row) may ellipsize; not the row itself or the status line.
        let first = row.find(clip);
        let id_line = row.find(".min_w_0()").expect("id line");
        assert!(
            first.is_none_or(|p| p > id_line),
            "{clip} on the row or on the status/button line would clip name/status/button"
        );
    }
}

#[test]
fn selected_device_id_is_shown_in_full() {
    // The list ellipsizes long IDs; the active flow is where the rest is read, so that line must wrap.
    let line = VIEW.lines().find(|l| l.contains("\"Dispositivo: {}\"")).expect("selected device line");
    assert!(!line.contains(".truncate()"), "selected device ID must wrap, not ellipsize");
}

#[test]
fn long_ids_ellipsize_on_their_own_line() {
    let row = row_source();
    let start = row.find(".min_w_0()").expect("id line");
    let id_line = &row[start..start + row[start..].find("device.device_id").unwrap()];
    assert!(id_line.contains(".truncate()"), "long ID must ellipsize");
    assert!(id_line.contains(".min_w_0()"), "flex child needs min_w_0 or the ID stretches the row");
    assert!(id_line.contains("one_line("), "newlines in an ID must not add rows");
    assert!(
        row.find("\"Verifica\"").unwrap() < start,
        "ID must come after, not share the line with, the status/button line"
    );
}

#[test]
fn unverified_status_is_prominent_with_action_and_verified_is_not() {
    let row = row_source();
    assert!(row.contains("SEMIBOLD"), "status must be bold");
    assert!(row.contains("⚠ Non verificato") && row.contains("✓ Verificato"));
    assert!(row.contains("t.error"), "unverified must use the error colour");
    let action = row.find("\"Verifica\"").expect("verify action");
    assert!(
        row[..action].contains("when(!device.verified"),
        "Verifica button only for unverified rows"
    );
}

#[test]
fn scroll_area_scrolls_vertically_and_can_shrink() {
    let at = VIEW.find("\"verification-content\"").expect("scroll area");
    let area = &VIEW[at..at + 400];
    assert!(area.contains(".overflow_y_scroll()"));
    assert!(area.contains(".min_h_0()"), "without min_h_0 the list grows past the window");
    assert!(area.contains(".flex_1()"));
}

#[test]
fn removed_explanatory_copy_is_gone_everywhere() {
    for (file, src) in ALL_SOURCES {
        for phrase in REMOVED {
            if file.starts_with("tests/") {
                // Tests may name the phrase only inside a negative assertion.
                continue;
            }
            assert!(!src.contains(phrase), "{file} still contains {phrase:?}");
        }
    }
    for (file, src) in ALL_SOURCES.iter().filter(|(f, _)| f.starts_with("tests/")) {
        for line in src.lines().filter(|l| REMOVED.iter().any(|p| l.contains(p))) {
            assert!(
                line.contains("!") || line.contains("contains"),
                "{file}: positive reference to a removed phrase: {line}"
            );
        }
    }
}

fn devices(n: usize, verified: impl Fn(usize) -> bool, id: impl Fn(usize) -> String) -> Vec<VerificationDevice> {
    (0..n)
        .map(|i| VerificationDevice {
            user_id: "@alice:matrix-desktop.test".into(),
            device_id: id(i),
            verified: verified(i),
        })
        .collect()
}

fn rows_tree(list: Vec<VerificationDevice>) -> Vec<(accesskit::NodeId, accesskit::Node)> {
    let account = AccountId::new(7);
    let mut model = DeviceVerification::new(account);
    model.apply(VerificationEvent::Devices { account_id: account, devices: list });
    let rows = DeviceNodes::default().project(account, &model);
    DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 6])
        .with_devices(rows)
        .tree()
        .nodes
}

fn has_value(nodes: &[(accesskit::NodeId, accesskit::Node)], v: &str) -> bool {
    nodes.iter().any(|(_, n)| n.role() == Role::Label && n.value() == Some(v))
}

#[test]
fn a11y_tree_has_title_refresh_and_per_row_name_status_and_action_without_explanation() {
    let nodes = rows_tree(devices(2, |i| i == 0, |i| format!("DEV{i}")));
    assert_eq!(nodes[0].1.label(), Some("Verifica dispositivi"));
    assert!(has_value(&nodes, "Altri dispositivi"));
    assert!(has_value(&nodes, "DEV0 · Verificato") && has_value(&nodes, "DEV1 · Non verificato"));
    assert!(nodes.iter().any(|(_, n)| n.supports_action(Action::Click) && n.label() == Some("Aggiorna dispositivi")));
    let row = nodes.iter().find(|(_, n)| n.label() == Some("Avvia verifica con DEV1 · Non verificato"));
    assert!(row.expect("unverified row").1.supports_action(Action::Click));
    // Verified rows: read-only label above, no clickable "Avvia verifica" node.
    assert!(!nodes.iter().any(|(_, n)| n.label().is_some_and(|l| l.contains("DEV0"))));
    for (_, n) in &nodes {
        for text in [n.label(), n.value()].into_iter().flatten() {
            assert!(!REMOVED.iter().any(|p| text.contains(p)), "removed copy exposed: {text}");
        }
    }
}

#[test]
fn twenty_five_devices_all_exposed_with_long_ids_and_stable_distinct_nodes() {
    let long = |i: usize| format!("DEVICE_{i:02}_{}", "X".repeat(300));
    let nodes = rows_tree(devices(25, |i| i % 2 == 0, long));
    let rows: Vec<_> = nodes.iter().filter(|(_, n)| n.label().is_some_and(|l| l.starts_with("Avvia verifica con DEVICE_"))).collect();
    assert_eq!(rows.len(), 12, "every unverified device needs its own actionable node");
    assert!(rows.iter().all(|(_, n)| n.supports_action(Action::Click)));
    let ids: std::collections::HashSet<_> = rows.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids.len(), 12);
    let labels = nodes.iter().filter(|(_, n)| n.role() == Role::Label && n.value().is_some_and(|v| v.starts_with("DEVICE_"))).count();
    assert_eq!(labels, 25, "all 25 devices stay readable");
    assert!(rows.iter().all(|(_, n)| n.label().unwrap().len() > 300), "a11y must not truncate the ID");
}

#[test]
fn empty_list_says_so() {
    assert!(has_value(&rows_tree(vec![]), "Nessun altro dispositivo disponibile"));
}

#[test]
fn verified_row_is_not_clickable_in_view_and_model() {
    let row = row_source();
    let when = row.find("when(!device.verified, |d| d.cursor_pointer()").expect("conditional click");
    assert!(!row[..when].contains("cursor_pointer"), "verified row must not look clickable");
    assert!(VIEW[VIEW.find(".on_click(cx.listener(move").unwrap()..].starts_with(".on_click"));
    let account = AccountId::new(7);
    let mut model = DeviceVerification::new(account);
    model.apply(VerificationEvent::Devices { account_id: account, devices: devices(2, |i| i == 0, |i| format!("D{i}")) });
    assert!(!model.begin(0), "verified device must not start a SAS");
    assert!(model.begin(1));
}

#[test]
fn own_device_row_is_a_label_shown_once_and_never_clickable() {
    let account = AccountId::new(7);
    let mut model = DeviceVerification::new(account);
    model.apply(VerificationEvent::Profile { account_id: account, user_id: "@a:x".into(), display_name: None, device_id: Some("OWN".into()) });
    model.apply(VerificationEvent::Devices { account_id: account, devices: devices(2, |_| false, |i| format!("D{i}")) });
    let rows = DeviceNodes::default().project(account, &model);
    let nodes = DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 6]).with_devices(rows).tree().nodes;
    let own: Vec<_> = nodes.iter().filter(|(_, n)| [n.label(), n.value()].into_iter().flatten().any(|t| t.contains("OWN"))).collect();
    assert_eq!(own.len(), 1, "own device appears exactly once");
    assert_eq!(own[0].1.value(), Some("Questo dispositivo · OWN"));
    assert!(!own[0].1.supports_action(Action::Click));
    let first = nodes.iter().skip(1).find(|(_, n)| n.role() == Role::Label).unwrap();
    assert_eq!(first.1.value(), Some("Questo dispositivo · OWN"), "own device comes first");
}
