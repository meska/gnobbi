//! The one switch row (label left, switch right). Callers add `.on_click`, `.track_focus` and the a11y node.
use gpui::{ElementId, IntoElement, Stateful, Div, InteractiveElement, ParentElement, Styled, div, prelude::*, px, rgb};
use matrix_desktop::DesignTokens;

/// A disabled switch is always drawn off, whatever `on` says.
fn drawn_on(on: bool, enabled: bool) -> bool {
    on && enabled
}

pub(crate) fn switch_row(
    id: impl Into<ElementId>,
    label: impl IntoElement,
    on: bool,
    enabled: bool,
    focused: bool,
    t: DesignTokens,
) -> Stateful<Div> {
    let on = drawn_on(on, enabled);
    div()
        .id(id)
        .min_h(px(44.))
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(if focused { t.accent } else { t.surface }))
        .text_color(rgb(if enabled { t.text } else { t.secondary }))
        .when(enabled, |d| d.cursor_pointer().hover(|h| h.bg(rgb(t.selected))))
        .when(!enabled, |d| d.opacity(0.5))
        // The label takes the leftover width and truncates; the switch never shrinks.
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(
            div()
                .flex_shrink_0()
                .w(px(36.))
                .h(px(20.))
                .rounded_full()
                .flex()
                .items_center()
                .px(px(2.))
                .bg(rgb(if on { t.accent } else { t.disabled }))
                .when(on, |d| d.justify_end())
                .child(div().size(px(16.)).rounded_full().bg(rgb(t.on_accent))),
        )
}

#[cfg(test)]
mod tests {
    use super::drawn_on;

    #[test]
    fn on_off_and_disabled() {
        assert!(drawn_on(true, true));
        assert!(!drawn_on(false, true));
        assert!(!drawn_on(true, false));
        assert!(!drawn_on(false, false));
    }
}
