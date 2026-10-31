//! GPUI rendering of a [`SearchList`]: avatar, title, one detail line, the active row highlighted,
//! «Altri risultati» last, and the one-line status. Shared by every dialog with a search field.
use crate::*;
use matrix_desktop::search_list::{Hit, Row, SearchList, Status, more_text};

/// One element per row. `on_choose(view, i, cx)` runs on click; the caller sets the active row
/// and acts (a hit starts its action, the «Altri risultati» row asks the driver for the next page).
pub(crate) fn rows<H: Hit, V: 'static>(
    id: &'static str,
    list: &SearchList<H>,
    t: matrix_desktop::DesignTokens,
    avatars: &avatar_view::Avatars,
    cx: &mut Context<V>,
    on_choose: fn(&mut V, usize, &mut Context<V>),
) -> Vec<gpui::AnyElement> {
    let active = list.active();
    list.rows()
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let line =
                |text: String, color: u32| div().truncate().text_color(rgb(color)).child(text);
            let base = div()
                .id((id, i))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(40.))
                .cursor_pointer()
                .when(i == active, |d| d.bg(rgb(t.input)))
                .on_click(cx.listener(move |this, _, _, cx| on_choose(this, i, cx)));
            match row {
                Row::More(more) => base
                    .child(line(more_text(more).to_owned(), t.disabled))
                    .into_any_element(),
                Row::Hit(hit) => {
                    let initial = hit
                        .title()
                        .trim_start_matches('@')
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default();
                    let picture = match avatars.get(hit.avatar()) {
                        Some(image) => avatar_view::round(image, 28.),
                        None => div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(t.accent))
                            .text_color(rgb(t.on_accent))
                            .child(initial)
                            .into_any_element(),
                    };
                    let detail = hit.detail();
                    base.child(picture)
                        .child(if detail.is_empty() {
                            div().min_w_0().child(line(hit.label(), t.text))
                        } else {
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(line(hit.title().to_owned(), t.text))
                                .child(line(detail, t.disabled).text_size(px(12.)))
                        })
                        .into_any_element()
                }
            }
        })
        .collect()
}

pub(crate) fn status<H: Hit>(
    list: &SearchList<H>,
    t: matrix_desktop::DesignTokens,
) -> Option<gpui::AnyElement> {
    list.status().map(|status| {
        div()
            .text_size(px(13.))
            .text_color(rgb(if matches!(status, Status::Error(_)) {
                t.error
            } else {
                t.disabled
            }))
            .child(status.text())
            .into_any_element()
    })
}
