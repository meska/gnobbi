#[test]
fn update_switch_has_runtime_localization() {
    let view = include_str!("../src/settings_view.rs");
    assert!(view.contains("update_check_label()"), "visible switch must use the runtime locale");
    let a11y = include_str!("../src/accessibility/settings.rs");
    assert!(a11y.contains("update_check_label()"), "screen-reader label must use the runtime locale");
}
