//! One button of the list filter row: icon from the composer family, name as hover and accessible name,
//! count as data. Selected = filled accent background (not colour alone). The caller adds the click,
//! the focus handling and the keyboard ring (`input_modality`), so the row can be a single tab stop.
use crate::composer_icons::{Icon, icon};
use crate::rail_view::RailTooltip;
use crate::*;
use matrix_desktop::{FilterButton, FilterId};

/// Minimum click target, logical px.
pub(crate) const TARGET: f32 = 32.;
const _: () = assert!(TARGET >= 32.);

pub(crate) fn filter_icon_kind(id: &FilterId) -> Icon {
    match id {
        FilterId::All => Icon::FilterAll,
        FilterId::Group(tag) if tag == "m.favourite" => Icon::Star,
        FilterId::Group(_) => Icon::LowPriority,
        FilterId::GroupsMenu => Icon::Folder,
        FilterId::Unread => Icon::Dot,
    }
}

pub(crate) fn filter_icon(
    element_id: impl Into<gpui::ElementId>,
    button: &FilterButton,
    theme: Theme,
    ring: bool,
) -> gpui::Stateful<gpui::Div> {
    let t = theme.tokens();
    let (fg, label) = (if button.selected { t.on_accent } else { t.secondary }, button.name.clone());
    div()
        .id(element_id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap_1()
        .min_w(px(TARGET))
        .h(px(TARGET))
        .px_1()
        .rounded_md()
        .cursor_pointer()
        .border_1()
        .border_color(if ring { gpui::Hsla::from(rgb(t.accent)) } else { gpui::transparent_black() })
        .when(button.selected, |d| d.bg(rgb(t.accent)))
        .text_color(rgb(fg))
        .text_size(px(12.))
        .child(icon(filter_icon_kind(&button.id), fg))
        .children(button.count.map(|n| n.to_string()))
        .tooltip(move |_, cx| cx.new(|_| RailTooltip { label: label.clone(), theme }).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_has_an_icon() {
        let a = matrix_desktop::Sidebar::demo(matrix_domain::AccountId::new(1));
        for b in a.filter_buttons() {
            filter_icon_kind(&b.id);
        }
    }
}
