//! Every drawer row uses the plain text colour; accent is only for the focus ring and the active-account dot.
const SETTINGS: &str = include_str!("../src/settings_view.rs");

#[test]
fn drawer_rows_share_one_text_colour() {
    let body = SETTINGS.split("fn render_drawer_body").nth(1).unwrap();
    let rows = body.split("row(\"").skip(1).take(6);
    let mut n = 0;
    for r in rows {
        let r = r.split(".on_click").next().unwrap();
        assert!(r.contains(".text_color(rgb(t.text))"), "row {} not t.text", r.split('"').next().unwrap());
        n += 1;
    }
    assert_eq!(n, 6);
}
