//! Colours live in the theme model (`lib.rs`): UI files must not hardcode them.
use std::fs;

#[test]
fn ui_sources_hold_no_literal_colours() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut found = Vec::new();
    for entry in fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // lib.rs is the theme module; *_test(s).rs are test-only code.
        if path.extension().is_none_or(|e| e != "rs") || name == "lib.rs" || name.contains("_test") {
            continue;
        }
        for (n, line) in fs::read_to_string(&path).unwrap().lines().enumerate() {
            if line.contains("rgb(0x") || line.contains("rgba(0x") || line.contains("hsla(") || line.contains("hsl(") || line.contains("color: 0x") {
                found.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "literal colours outside lib.rs:\n{}", found.join("\n"));
}
