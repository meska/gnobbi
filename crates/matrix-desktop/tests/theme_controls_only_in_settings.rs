//! Light/dark choice lives only in Settings: no theme control or «Modalità notte» row anywhere else (drawer included).
//! shortcut: source-level check, upgrade to a rendered-tree assert when one exists.
use std::fs;

#[test]
fn no_theme_label_outside_settings_and_preferences() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let banned = ["Modalità notte", "Modalità scura", "Tema scuro", "Tema chiaro", "Passa al tema", "Notte automatica", "\"Scuro\"", "\"Chiaro\""];
    let mut found = Vec::new();
    for entry in fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // settings_view.rs owns the controls, preferences.rs the theme names; *_test(s).rs are test-only code.
        if path.extension().is_none_or(|e| e != "rs") || ["settings_view.rs", "preferences.rs"].contains(&name.as_str()) || name.contains("_test") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(b) = banned.iter().find(|b| line.contains(**b)) {
                found.push(format!("{name}:{}: {b}", n + 1));
            }
        }
    }
    assert!(found.is_empty(), "theme controls outside Settings:\n{}", found.join("\n"));
}

#[test]
fn drawer_body_scrolls_so_settings_stay_reachable_on_short_windows() {
    let menu = include_str!("../src/menu_view.rs");
    assert!(menu.contains(".id(\"drawer-body\")") && menu.contains(".overflow_y_scroll()"), "drawer/settings body must scroll");
}
