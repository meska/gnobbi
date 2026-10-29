//! Persistent desktop grouping navigation; no network operations or synthetic Spaces.
use crate::*;

// Rail entry geometry in one place; indicator/badge offsets follow the icon (36x32, label 11 px).
const RAIL_ENTRY_HEIGHT: f32 = 63.;
const RAIL_INDICATOR_TOP: f32 = 8.;
const RAIL_BADGE_TOP: f32 = 1.;

const RAIL_ICON_PX: f32 = 34.;
const RAIL_LABEL_UNITS: usize = 10;

// shortcut: width is estimated (wide chars count 2), not measured; measure text if fonts change.
fn rail_label(name: &str) -> String {
    let w = |c: char| {
        if c.is_ascii() || (c as u32) < 0x2000 {
            1
        } else {
            2
        }
    };
    if name.chars().map(w).sum::<usize>() <= RAIL_LABEL_UNITS {
        return name.to_owned();
    }
    let (mut used, mut out) = (0, String::new());
    for c in name.chars() {
        used += w(c);
        if used > RAIL_LABEL_UNITS - 1 {
            break;
        }
        out.push(c);
    }
    format!("{}…", out.trim_end())
}

struct RailTooltip {
    label: String,
    theme: Theme,
}
impl Render for RailTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.tokens();
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .font_family(matrix_desktop::UI_FONT)
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

/// Where focus sits around the «Modifica» button and its panel.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EditFocus {
    Rail,
    Button,
    Panel,
    Theme,
}

/// Keyboard transition for the edit button/panel: (new focus, panel open); None = not ours.
fn edit_step(at: EditFocus, open: bool, key: &str, shift: bool) -> Option<(EditFocus, bool)> {
    match (at, key) {
        (EditFocus::Button, "enter" | "space") if open => Some((EditFocus::Button, false)),
        (EditFocus::Button, "enter" | "space") => Some((EditFocus::Panel, true)),
        (EditFocus::Button, "tab") if shift => Some((EditFocus::Rail, open)),
        (EditFocus::Button, "tab") => Some((EditFocus::Theme, open)),
        (EditFocus::Button | EditFocus::Panel, "escape") => Some((EditFocus::Button, false)),
        (EditFocus::Panel, "tab") => Some((EditFocus::Panel, true)), // empty panel: nothing to tab to
        _ => None,
    }
}

pub(crate) struct RailUi {
    pub(crate) focus: FocusHandle,
    pub(crate) edit_focus: FocusHandle,
    edit_panel_focus: FocusHandle,
    edit_open: bool,
    edit_cursor: usize,
    pub(crate) layouts: matrix_desktop::RailLayouts,
    pub(crate) target: Option<RoomScope>,
    scroll: gpui::ScrollHandle,
    anchors: std::cell::RefCell<HashMap<Option<RoomScope>, gpui::ScrollAnchor>>,
}
impl RailUi {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            focus: cx.focus_handle(),
            edit_focus: cx.focus_handle(),
            edit_panel_focus: cx.focus_handle(),
            edit_open: false,
            edit_cursor: 0,
            layouts: Default::default(),
            target: None,
            scroll: gpui::ScrollHandle::new(),
            anchors: Default::default(),
        }
    }
}
/// What a key does inside the edit panel: move the cursor, move the space, hide/show it.
#[derive(Debug, PartialEq)]
enum PanelAct {
    Go(usize),
    Move(isize),
    Toggle,
}

fn panel_act(key: &str, shift: bool, cursor: usize, len: usize) -> Option<PanelAct> {
    match (key, shift) {
        ("up", true) => Some(PanelAct::Move(-1)),
        ("down", true) => Some(PanelAct::Move(1)),
        ("up", false) => Some(PanelAct::Go(cursor.saturating_sub(1))),
        ("down", false) => Some(PanelAct::Go((cursor + 1).min(len.saturating_sub(1)))),
        ("enter" | "space", _) if len > 0 => Some(PanelAct::Toggle),
        _ => None,
    }
}

impl Shell {
    fn rail_account(&self) -> u64 {
        self.active_id.map_or(0, |id| id.as_u64())
    }

    /// Every space in the user's order with its hidden flag; «Tutte» first.
    fn rail_arranged(&self) -> Vec<(matrix_desktop::NavigationEntry, bool)> {
        self.rail_ui
            .layouts
            .get(self.rail_account())
            .arrange(self.sidebar.navigation_entries())
    }

    /// What the rail shows: the arranged spaces minus the hidden ones.
    pub(crate) fn rail_entries(&self) -> Vec<matrix_desktop::NavigationEntry> {
        self.rail_arranged().into_iter().filter(|(_, hidden)| !hidden).map(|(e, _)| e).collect()
    }

    /// Applies a change to this account's layout and saves it; a failed save keeps the change for this run.
    fn rail_layout_edit(&mut self, change: impl FnOnce(&mut matrix_desktop::RailLayout, &[String])) {
        let account = self.rail_account();
        let mut layout = self.rail_ui.layouts.get(account);
        let current: Vec<String> = self
            .rail_arranged()
            .into_iter()
            .filter_map(|(e, _)| e.scope.map(|s| s.room_id.as_str().to_owned()))
            .collect();
        change(&mut layout, &current);
        self.rail_ui.layouts.set(account, layout);
        if let Some(dirs) = &self.prefs_dirs {
            let _ = self.rail_ui.layouts.save(dirs);
        }
    }

    /// Edit-panel action on the space under the cursor (row = position among the spaces).
    fn rail_edit_act(&mut self, row: usize, act: PanelAct) {
        let spaces: Vec<_> = self.rail_arranged().into_iter().skip(1).collect();
        let Some((entry, _)) = spaces.get(row) else {
            return;
        };
        let (id, scope) = match &entry.scope {
            Some(s) => (s.room_id.as_str().to_owned(), s.clone()),
            None => return,
        };
        match act {
            PanelAct::Go(to) => self.rail_ui.edit_cursor = to,
            PanelAct::Move(delta) => {
                self.rail_layout_edit(|l, cur| {
                    l.shift(cur, &id, delta);
                });
                self.rail_ui.edit_cursor = row.saturating_add_signed(delta).min(spaces.len() - 1);
            }
            PanelAct::Toggle => {
                self.rail_layout_edit(|l, _| l.toggle_hidden(&id));
                if self.sidebar.active_space() == Some(&scope) && !self.rail_entries().iter().any(|e| e.scope.as_ref() == Some(&scope)) {
                    self.sidebar.set_space(None);
                }
            }
        }
    }

    pub(crate) fn reveal_rail_target(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(anchor) = self.rail_ui.anchors.borrow().get(&self.rail_ui.target) {
            anchor.scroll_to(window, cx);
        }
    }

    fn rail_observer(
        &self,
        _scope: &Option<RoomScope>,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let view = self.semantic_spaces(_window)?;
            let id = view.choices.iter().find(|c| c.scope == *_scope)?.id;
            Some(self.space_observer(id, _cx))
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            #[cfg(test)]
            {
                crate::sidebar_navigation_test::observe(format!("rail-{_scope:?}"))
            }
            #[cfg(not(test))]
            {
                None
            }
        }
    }

    pub(crate) fn rail_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        if window.viewport_size().width < px(800.)
            || self.notifications_open
            || self.verification_open
            || self.room_details_open
        {
            return false;
        }
        let at = if self.rail_ui.edit_focus.is_focused(window) {
            Some(EditFocus::Button)
        } else if self.rail_ui.edit_panel_focus.is_focused(window) {
            Some(EditFocus::Panel)
        } else {
            None
        };
        if at == Some(EditFocus::Panel)
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && let len = self.rail_arranged().len().saturating_sub(1)
            && let Some(act) = panel_act(&key.key, key.modifiers.shift, self.rail_ui.edit_cursor, len)
        {
            if !(matches!(act, PanelAct::Toggle) && event.is_held) {
                let row = self.rail_ui.edit_cursor.min(len.saturating_sub(1));
                self.rail_edit_act(row, act);
            }
            cx.notify();
            return true;
        }
        if let Some(at) = at
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && !event.is_held
            && let Some((to, open)) = edit_step(at, self.rail_ui.edit_open, &key.key, key.modifiers.shift)
        {
            self.rail_ui.edit_open = open;
            match to {
                EditFocus::Rail => self.rail_ui.focus.focus(window),
                EditFocus::Button => self.rail_ui.edit_focus.focus(window),
                EditFocus::Panel => self.rail_ui.edit_panel_focus.focus(window),
                EditFocus::Theme => self.theme_focus.focus(window),
            }
            cx.notify();
            return true;
        }
        if key.modifiers.control && key.modifiers.shift && key.key == "s" {
            self.rail_ui.target = self.sidebar.active_space().cloned();
            self.rail_ui.focus.focus(window);
            cx.notify();
            return true;
        }
        if !self.rail_ui.focus.is_focused(window)
            || key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
        {
            return false;
        }
        let entries = self.rail_entries();
        let index = entries
            .iter()
            .position(|entry| entry.scope == self.rail_ui.target)
            .unwrap_or(0);
        let last = entries.len() - 1;
        if key.key == "tab"
            && key.modifiers.shift
            && index == 0
            && self.focus_last_chat_control(window, cx)
        {
            return true;
        }
        let next = match key.key.as_str() {
            "down" => Some((index + 1).min(last)),
            "up" => Some(index.saturating_sub(1)),
            "home" => Some(0),
            "end" => Some(last),
            "tab" if key.modifiers.shift && index > 0 => Some(index - 1),
            "tab" if !key.modifiers.shift && index < last => Some(index + 1),
            "tab" if !key.modifiers.shift => {
                self.rail_ui.edit_focus.focus(window);
                None
            }
            "tab" | "escape" => {
                self.search_focus.focus(window);
                None
            }
            _ => return false,
        };
        if let Some(next) = next {
            self.rail_ui.target = entries[next].scope.clone();
            self.reveal_rail_target(window, cx);
        }
        cx.notify();
        true
    }

    /// Runs on the release of a completed Space/Enter gesture (see `sidebar_activation_up`).
    pub(crate) fn rail_activate(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        sidebar_navigation_test::ACTIVATIONS[2].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let entries = self.rail_entries();
        let index = entries
            .iter()
            .position(|entry| entry.scope == self.rail_ui.target)
            .unwrap_or(0);
        if self.sidebar.set_space(entries[index].scope.clone()) {
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        cx.notify();
    }

    pub(crate) fn render_rail(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        // The menu button stays on top of the rail, above «Tutte» and the scrolling entries.
        div()
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(t.input))
            .border_r_1()
            .border_color(rgb(t.border))
            .child(
                div()
                    .id("rail-menu")
                    .h(px(t.header))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.render_menu_button(t, window, cx)),
            )
            .child(
                div()
                    .id("navigation-rail")
                    .track_focus(&self.rail_ui.focus)
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.rail_ui.scroll)
                    .children(
                        self
                            .rail_entries()
                            .into_iter()
                            .enumerate()
                            .map(|(index, entry)| {
                                let scope = entry.scope;
                                let source_visit = self.sidebar.navigation_callback();
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                let callback = self.semantic_spaces(window).and_then(|view| {
                                    let choice = view.choices.iter().find(|c| c.scope == scope)?;
                                    Some(crate::spaces_view::space_callback(
                                matrix_desktop::accessibility::spaces::SpacesAction::Choose {
                                    revision: view.revision,
                                    account: view.account,
                                    id: choice.id,
                                    scope: scope.clone(),
                                    previous: view.active.clone(),
                                },
                            ))
                                });
                                let anchor = self
                                    .rail_ui
                                    .anchors
                                    .borrow_mut()
                                    .entry(scope.clone())
                                    .or_insert_with(|| {
                                        gpui::ScrollAnchor::for_handle(self.rail_ui.scroll.clone())
                                    })
                                    .clone();
                                let focused = self.rail_ui.focus.is_focused(window)
                                    && self.rail_ui.target == scope;
                                let initials: String = if scope.is_none() {
                                    "▦".into()
                                } else {
                                    entry.name.chars().take(2).collect()
                                };
                                let label = if scope.is_none() {
                                    "Tutte le conversazioni".to_owned()
                                } else {
                                    format!("Spazio Matrix: {}", entry.name)
                                };
                                let scope_key = scope.clone();
                                let pointer =
                            move |this: &mut Shell,
                                  window: &mut Window,
                                  cx: &mut Context<Shell>| {
                                this.navigation_wide = window.viewport_size().width >= px(800.);
                                this.refresh_navigation_visibility();
                                if !source_visit(&this.sidebar) {
                                    return;
                                }
                                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                {
                                    if let Some(action) = callback
                                        .as_ref()
                                        .and_then(|f| f(this.semantic_spaces(window).as_ref()))
                                    {
                                        this.apply_semantic_spaces(action, window, cx);
                                    }
                                }
                                #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                                if this.sidebar.set_space(scope.clone()) {
                                    this.rail_ui.target = scope.clone();
                                    this.rail_ui.focus.focus(window);
                                    this.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
                                    cx.notify();
                                }
                            };
                                #[cfg(test)]
                                let pointer =
                                    sidebar_navigation_test::capture_rail(&scope_key, pointer);
                                let theme = self.theme;
                                div()
                                    .id(("rail-entry", index))
                                    .relative()
                                    .anchor_scroll(Some(anchor))
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| RailTooltip {
                                            label: label.clone(),
                                            theme,
                                        })
                                        .into()
                                    })
                                    .w_full()
                                    .h(px(RAIL_ENTRY_HEIGHT))
                                    .flex_shrink_0()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .border_2()
                                    .border_color(rgb(if focused { t.accent } else { t.input }))
                                    .bg(rgb(if entry.selected { t.selected } else { t.input }))
                                    .text_color(rgb(if entry.selected {
                                        t.accent
                                    } else {
                                        t.secondary
                                    }))
                                    .cursor_pointer()
                                    .children(self.rail_observer(&scope_key, window, cx))
                                    .child(match self.avatars.get(entry.avatar.as_deref()) {
                                        Some(image) => avatar_view::round(image, RAIL_ICON_PX),
                                        None => div()
                                            .size(px(RAIL_ICON_PX))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .bg(rgb(t.surface))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(16.))
                                            .child(initials)
                                            .into_any_element(),
                                    })
                                    .child(
                                        div()
                                            .w_full()
                                            .px_1()
                                            .truncate()
                                            .text_center()
                                            .text_size(px(11.))
                                            .child(rail_label(&entry.name)),
                                    )
                                    .when(entry.selected, |d| {
                                        d.child(
                                            div()
                                                .absolute()
                                                .left_0()
                                                .top(px(RAIL_INDICATOR_TOP))
                                                .w(px(3.))
                                                .h(px(28.))
                                                .bg(rgb(t.accent)),
                                        )
                                    })
                                    .when(entry.unread > 0, |d| {
                                        d.child(
                                            div()
                                                .absolute()
                                                .right(px(3.))
                                                .top(px(RAIL_BADGE_TOP))
                                                .rounded_full()
                                                .px_1()
                                                .bg(rgb(t.accent))
                                                .text_color(rgb(t.on_accent))
                                                .text_size(px(10.))
                                                .child(if entry.unread > 99 {
                                                    "99+".into()
                                                } else {
                                                    entry.unread.to_string()
                                                }),
                                        )
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        pointer(this, window, cx)
                                    }))
                            }),
                    ),
            )
            .child(self.render_edit_button(window, cx))
            .into_any_element()
    }

    fn render_edit_button(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let focused = self.rail_ui.edit_focus.is_focused(window);
        div()
            .id("rail-edit")
            .track_focus(&self.rail_ui.edit_focus)
            .w_full()
            .h(px(40.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(rgb(if focused { t.accent } else { t.input }))
            .bg(rgb(if self.rail_ui.edit_open { t.selected } else { t.input }))
            .text_color(rgb(t.accent))
            .text_size(px(12.))
            .cursor_pointer()
            .child("Modifica")
            .on_click(cx.listener(|this, _, window, cx| {
                this.rail_ui.edit_open = !this.rail_ui.edit_open;
                let focus = if this.rail_ui.edit_open {
                    &this.rail_ui.edit_panel_focus
                } else {
                    &this.rail_ui.edit_focus
                };
                focus.focus(window);
                cx.notify();
            }))
            .into_any_element()
    }

    pub(crate) fn open_rail_edit(&mut self, window: &mut Window) {
        self.rail_ui.edit_open = true;
        self.rail_ui.edit_panel_focus.focus(window);
    }

    pub(crate) fn rail_edit_open(&self) -> bool {
        self.rail_ui.edit_open
    }

    pub(crate) fn render_edit_panel(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let focused = self.rail_ui.edit_panel_focus.is_focused(window);
        let spaces: Vec<_> = self.rail_arranged().into_iter().skip(1).collect();
        let last = spaces.len().saturating_sub(1);
        let cursor = self.rail_ui.edit_cursor.min(last);
        let action = |id: (&'static str, usize), label: &'static str, color: u32| {
            div().id(id).px_2().py_1().rounded_md().text_size(px(12.)).text_color(rgb(color)).cursor_pointer().child(label)
        };
        div()
            .id("rail-edit-panel")
            .track_focus(&self.rail_ui.edit_panel_focus)
            .absolute()
            .left(px(72.))
            .top_0()
            .bottom_0()
            .w(window.viewport_size().width.min(px(320.)))
            .bg(rgb(t.surface))
            .border_r_1()
            .border_color(rgb(if focused { t.accent } else { t.border }))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .child(
                div()
                    .id("rail-edit-groups")
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(14.))
                    .text_color(rgb(t.accent))
                    .child("Gruppi")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.rail_ui.edit_open = false;
                        this.open_groups_edit(window, cx);
                    })),
            )
            .children(spaces.into_iter().enumerate().map(|(row, (entry, hidden))| {
                let on = focused && row == cursor;
                let color = if hidden { t.secondary } else { t.text };
                div()
                    .id(("rail-edit-row", row))
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_2()
                    .border_color(rgb(if on { t.accent } else { t.surface }))
                    .text_color(rgb(color))
                    .text_size(px(14.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.rail_ui.edit_cursor = row;
                        cx.notify();
                    }))
                    .child(div().flex_1().truncate().child(entry.name))
                    .child(action(("rail-edit-up", row), "↑", t.accent).on_click(cx.listener(move |this, _, _, cx| {
                        this.rail_edit_act(row, PanelAct::Move(-1));
                        cx.notify();
                    })))
                    .child(action(("rail-edit-down", row), "↓", t.accent).on_click(cx.listener(move |this, _, _, cx| {
                        this.rail_edit_act(row, PanelAct::Move(1));
                        cx.notify();
                    })))
                    .child(
                        action(("rail-edit-hide", row), if hidden { "Mostra" } else { "Nascondi" }, t.accent).w(px(76.)).text_center().on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.rail_edit_act(row, PanelAct::Toggle);
                                cx.notify();
                            }),
                        ),
                    )
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod edit_focus_tests {
    use super::{EditFocus::*, edit_step};

    #[test]
    fn open_close_and_return_focus() {
        assert_eq!(edit_step(Button, false, "enter", false), Some((Panel, true)));
        assert_eq!(edit_step(Button, false, "space", false), Some((Panel, true)));
        assert_eq!(edit_step(Panel, true, "escape", false), Some((Button, false)));
        assert_eq!(edit_step(Button, true, "enter", false), Some((Button, false)));
        assert_eq!(edit_step(Button, true, "escape", false), Some((Button, false)));
    }

    #[test]
    fn panel_keys() {
        use super::{PanelAct, panel_act};
        assert_eq!(panel_act("down", false, 0, 3), Some(PanelAct::Go(1)));
        assert_eq!(panel_act("down", false, 2, 3), Some(PanelAct::Go(2)));
        assert_eq!(panel_act("up", false, 0, 3), Some(PanelAct::Go(0)));
        assert_eq!(panel_act("up", true, 1, 3), Some(PanelAct::Move(-1)));
        assert_eq!(panel_act("down", true, 1, 3), Some(PanelAct::Move(1)));
        assert_eq!(panel_act("enter", false, 0, 3), Some(PanelAct::Toggle));
        assert_eq!(panel_act("space", false, 0, 0), None);
        assert_eq!(panel_act("x", false, 0, 3), None);
    }

    #[test]
    fn tab_order_around_the_button() {
        assert_eq!(edit_step(Button, false, "tab", true), Some((Rail, false)));
        assert_eq!(edit_step(Button, false, "tab", false), Some((Theme, false)));
        assert_eq!(edit_step(Panel, true, "tab", false), Some((Panel, true)));
        assert_eq!(edit_step(Panel, true, "tab", true), Some((Panel, true)));
        assert_eq!(edit_step(Button, false, "x", false), None);
    }
}

#[cfg(test)]
mod rail_label_tests {
    use super::rail_label;

    #[test]
    fn truncates_with_final_ellipsis() {
        for (name, want) in [
            ("Casa", "Casa"),
            ("Softair", "Softair"),
            ("Direct Messages", "Direct Me…"),
            ("Softair Team", "Softair T…"),
            ("🎮🎮🎮🎮🎮🎮", "🎮🎮🎮🎮…"),
            ("🎮🎮🎮🎮🎮", "🎮🎮🎮🎮🎮"),
        ] {
            assert_eq!(rail_label(name), want, "{name}");
        }
    }
}
