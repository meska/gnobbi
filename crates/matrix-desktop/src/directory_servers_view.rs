//! Server picker for the public-room directory: a button with the chosen server and, open, the
//! menu below it («<server> — Il tuo server» checked first). The owner routes keys to
//! `ServerPicker::key`, handles `Choice`, and shows the name field while `is_adding()`.
use crate::*;
use matrix_desktop::directory_servers::{Row, ServerPicker};

#[allow(dead_code)] // hosted by the join dialog, which owns the picker
/// `on_toggle` opens/closes the menu, `on_choose(i)` picks row `i`, `on_close` is a click outside.
pub(crate) fn render_server_picker(
    picker: &ServerPicker,
    t: matrix_desktop::DesignTokens,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    on_choose: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let button = div()
        .id("directory-server")
        .px_3()
        .py_2()
        .flex()
        .gap_2()
        .rounded_md()
        .cursor_pointer()
        .border_1()
        .border_color(rgb(t.border))
        .text_color(rgb(t.text))
        .child(format!("{} ▾", picker.shown()))
        .on_click(move |_, window, cx| on_toggle(window, cx));
    let menu = picker.is_open().then(|| {
        let on_choose = on_choose.clone();
        let highlighted = picker.highlighted();
        let rows = picker.rows();
        let item = |i: usize, row: &Row| {
            let on_choose = on_choose.clone();
            let mark = if picker.is_selected(row) { "✓ " } else { "   " };
            div()
                .id(("directory-server-item", i))
                .px_4()
                .py_2()
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(rgb(t.text))
                .when(i == highlighted, |d| d.bg(rgb(t.border)))
                .hover(|s| s.bg(rgb(t.border)))
                .child(format!(
                    "{}{}",
                    if matches!(row, Row::Server(_)) { mark } else { "" },
                    picker.label(row)
                ))
                .on_click(move |_, window, cx| on_choose(i, window, cx))
        };
        gpui::deferred(
            gpui::anchored().snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("directory-server-menu")
                    .occlude()
                    .flex()
                    .flex_col()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(t.border))
                    .bg(rgb(t.surface))
                    .min_w(px(220.))
                    .on_mouse_down_out(move |_, window, cx| on_close(window, cx))
                    .children(rows.iter().enumerate().map(|(i, r)| item(i, r))),
            ),
        )
        .with_priority(2)
    });
    div().child(button).children(menu).into_any_element()
}
