//! ⋮ menu of the list header: only entries backed by a real function.
//! Opens by click or Enter/Space, closes on Esc or outside click; focus returns to the button.
use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ListMenuEntry {
    RailEdit,
}

impl ListMenuEntry {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::RailEdit => "Modifica barra",
        }
    }
}

/// «Segna tutto come letto» (space rooms only) and «Dettagli spazio» are not offered: there is no
/// batch read marker or space panel yet. Add them here, under `in_space`, when those exist.
pub(crate) fn list_menu_entries(_in_space: bool) -> Vec<ListMenuEntry> {
    vec![ListMenuEntry::RailEdit]
}

impl Shell {
    pub(crate) fn list_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[0].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.set_list_menu_open(!self.list_menu_open, window, cx);
    }

    pub(crate) fn set_list_menu_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open && list_menu_entries(self.sidebar.active_space().is_some()).is_empty() {
            return;
        }
        self.list_menu_open = open;
        if open {
            self.list_menu_focus.focus(window)
        } else {
            self.theme_focus.focus(window)
        }
        cx.notify();
    }

    fn list_menu_run(&mut self, entry: ListMenuEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.list_menu_open = false;
        match entry {
            ListMenuEntry::RailEdit => self.open_rail_edit(window),
        }
        cx.notify();
    }

    /// Runs before every other key handler; true when the key was consumed.
    pub(crate) fn list_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.list_menu_open {
            return false;
        }
        let key = &event.keystroke;
        let entries = list_menu_entries(self.sidebar.active_space().is_some());
        match key.key.as_str() {
            "escape" => self.set_list_menu_open(false, window, cx),
            // shortcut: one entry today, so Enter/Space take the first; add Up/Down with the second.
            "enter" | "space" if !key.modifiers.modified() => {
                if let Some(entry) = entries.first() {
                    self.list_menu_run(*entry, window, cx)
                }
            }
            _ => (),
        }
        true
    }

    pub(crate) fn render_list_menu(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let left = if window.viewport_size().width >= px(800.) { 88. } else { 60. };
        let item = |i: usize, entry: ListMenuEntry| {
            div()
                .id(("list-menu-item", i))
                .px_4()
                .py_2()
                .cursor_pointer()
                .text_size(px(14.))
                .text_color(rgb(t.text))
                .hover(|s| s.bg(rgb(t.border)))
                .child(entry.label())
                .on_click(cx.listener(move |this, _, window, cx| this.list_menu_run(entry, window, cx)))
        };
        // The scrim takes every click, so the button underneath toggles it closed like any outside click.
        div()
            .id("list-menu-scrim")
            .track_focus(&self.list_menu_focus)
            .absolute()
            .inset_0()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| this.set_list_menu_open(false, window, cx)))
            .child(
                div()
                    .id("list-menu")
                    .absolute()
                    .left(px(left))
                    .top(px(t.header - 4.))
                    .min_w(px(180.))
                    .py_1()
                    .flex()
                    .flex_col()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(t.border))
                    .bg(rgb(t.surface))
                    .occlude()
                    .children(
                        list_menu_entries(self.sidebar.active_space().is_some())
                            .into_iter()
                            .enumerate()
                            .map(|(i, e)| item(i, e)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_only_the_real_functions_for_all_and_for_a_space() {
        for in_space in [false, true] {
            let labels: Vec<_> = list_menu_entries(in_space).iter().map(|e| e.label()).collect();
            assert_eq!(labels, ["Modifica barra"], "in_space={in_space}");
        }
    }
}
