//! «Ricerca rapida» overlay (Cmd+K / Ctrl+K, drawer entry): local search over the rooms already
//! known plus the «Visti di recente» / «Ricerche recenti» lists of `matrix_desktop::quick_search`.
//! Closes on Esc or a click outside, like the drawer. Keys are handled in `menu_key` (capture),
//! so room navigation underneath never sees them.
// Hook for «Utenti» / «Stanze pubbliche»: more `Source`s feeding `QuickSearch::hits`
// (server calls exist on main: `Client::search_users`, `operations::search_directory`).
use crate::*;
use matrix_desktop::quick_search::{Entry, Hit, Index, Recent, Source};
use std::ops::Range;

const ROW_H: f32 = 44.;
const LIST_MAX_H: f32 = 308.;
const LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zone {
    /// The horizontal «Visti di recente» row.
    Seen,
    /// The vertical list: «Ricerche recenti» while the field is empty, else the results.
    Rows,
}

pub(crate) struct QuickSearch {
    field: gpui::Entity<crate::login_field::LoginField>,
    index: Index,
    term: String,
    hits: Vec<Hit>,
    zone: Zone,
    active: usize,
    scroll: UniformListScrollHandle,
    seen_scroll: gpui::ScrollHandle,
    scrollbar: scrollbar_view::Scrollbar,
}

/// Where a navigation key puts the highlight; `None` when it does not move.
fn step(
    zone: Zone,
    active: usize,
    key: &str,
    seen: usize,
    rows: usize,
    typing: bool,
) -> Option<(Zone, usize)> {
    Some(match (zone, key) {
        (Zone::Seen, "left") => (zone, active.saturating_sub(1)),
        (Zone::Seen, "right") => (zone, (active + 1).min(seen.saturating_sub(1))),
        (Zone::Seen, "down") if rows > 0 => (Zone::Rows, 0),
        (Zone::Rows, "down") => (zone, (active + 1).min(rows.saturating_sub(1))),
        (Zone::Rows, "up") if active > 0 => (zone, active - 1),
        (Zone::Rows, "up") if !typing && seen > 0 => (Zone::Seen, 0),
        (_, "tab") if !typing && seen > 0 && rows > 0 => (
            if zone == Zone::Seen {
                Zone::Rows
            } else {
                Zone::Seen
            },
            0,
        ),
        _ => return None,
    })
}

/// «Svuota» from the keyboard: Backspace/Delete on the highlighted searches, or Cmd/Ctrl+Backspace.
fn clears(zone: Zone, key: &str, plain: bool, secondary: bool, typing: bool, rows: usize) -> bool {
    !typing
        && rows > 0
        && ((plain && zone == Zone::Rows && matches!(key, "backspace" | "delete"))
            || (secondary && key == "backspace"))
}

/// Name under a «Visti» avatar: one line that fits the 64 px cell, ellipsis at the end.
fn seen_label(name: &str) -> String {
    let name = matrix_desktop::row_text::one_line(name);
    // shortcut: counts chars, not pixels (wide glyphs may still clip); upgrade with text measuring.
    if name.chars().count() > 8 {
        name.chars().take(7).chain(['…']).collect()
    } else {
        name
    }
}

/// Row subtitle; a raw room ID (`!abc:server`) is never shown (the screen-reader label keeps it).
fn secondary_label(secondary: &str) -> &str {
    if secondary.starts_with('!') {
        ""
    } else {
        secondary
    }
}

/// «Nessun risultato» is text: `disabled` is a background token.
fn no_results_color(t: &matrix_desktop::DesignTokens) -> u32 {
    t.secondary
}

impl Shell {
    fn recents_account(&self) -> u64 {
        self.active_scope.account_id.as_u64()
    }

    fn save_recents(&self) {
        if let Some(dirs) = &self.prefs_dirs {
            let _ = self.recents.save(dirs);
        }
    }

    /// Any room that becomes the open one lands in «Visti di recente».
    pub(crate) fn note_seen(&mut self) {
        let Some(room) = self.sidebar.selected() else {
            return;
        };
        let item = Recent {
            id: room.scope.room_id.as_str().to_owned(),
            name: room.name.clone(),
            avatar: room.info.avatar.clone(),
        };
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        recents.opened(item);
        self.recents.set(account, recents);
        self.save_recents();
    }

    /// The vertical list as shown now.
    fn quick_rows(&self) -> Vec<Hit> {
        let Some(q) = &self.quick else {
            return Vec::new();
        };
        if !q.term.is_empty() {
            return q.hits.clone();
        }
        self.recents
            .get(self.recents_account())
            .searched()
            .iter()
            .map(|r| Hit {
                id: r.id.clone(),
                name: r.name.clone(),
                avatar: r.avatar.clone(),
                secondary: r.id.clone(),
            })
            .collect()
    }

    pub(crate) fn open_quick_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected || self.quick.is_some() {
            return;
        }
        self.set_menu_open(false, window, cx);
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field.placeholder = "Cerca";
            field
        });
        cx.observe(&field, |this, field, cx| {
            this.quick_typed(field.read(cx).input.display_text().trim().to_owned());
            cx.notify();
        })
        .detach();
        field.read(cx).focus.focus(window);
        // shortcut: the index is built once per opening (rooms arriving meanwhile are not
        // searched) and ordered like the list; upgrade with `Index::upsert` on room updates.
        let rooms = self.sidebar.rooms();
        let count = rooms.len() as u64;
        let index = Index::new(
            rooms
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.info.is_space && !r.info.invited)
                .map(|(i, r)| Entry::from_room(r, count - i as u64)),
        );
        let has_searched = !self
            .recents
            .get(self.recents_account())
            .searched()
            .is_empty();
        self.quick = Some(QuickSearch {
            field,
            index,
            term: String::new(),
            hits: Vec::new(),
            zone: if has_searched { Zone::Rows } else { Zone::Seen },
            active: 0,
            scroll: UniformListScrollHandle::new(),
            seen_scroll: gpui::ScrollHandle::new(),
            scrollbar: Default::default(),
        });
        cx.notify();
    }

    pub(crate) fn close_quick_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick = None;
        self.list_focus.focus(window);
        cx.notify();
    }

    fn quick_typed(&mut self, term: String) {
        let Some(q) = self.quick.as_mut().filter(|q| q.term != term) else {
            return;
        };
        q.hits = q.index.search(&term, LIMIT);
        q.term = term;
        q.zone = Zone::Rows;
        q.active = 0;
        q.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
    }

    /// Opens the room; a pick from the search lists (not from «Visti») is remembered too.
    fn quick_pick(&mut self, zone: Zone, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        let item = match zone {
            Zone::Seen => recents.seen().get(i).cloned(),
            Zone::Rows => self.quick_rows().get(i).map(Recent::from),
        };
        let Some(scope) = item.as_ref().and_then(|item| {
            let rooms = self.sidebar.rooms();
            rooms
                .iter()
                .find(|r| r.scope.room_id.as_str() == item.id)
                .map(|r| r.scope.clone())
        }) else {
            // The room is gone (left): drop the dead entry instead of a click that does nothing.
            if let Some(item) = item {
                recents.forget(&item.id);
                self.recents.set(account, recents);
                self.save_recents();
                if let Some(q) = self.quick.as_mut() {
                    q.active = q.active.saturating_sub(1);
                }
                cx.notify();
            }
            return;
        };
        if zone == Zone::Rows {
            recents.picked(item.unwrap());
            self.recents.set(account, recents);
            self.save_recents();
        }
        self.quick = None;
        if !self.sidebar.select(&scope) {
            // Hidden by an empty-room, unread or text filter: lift them rather than not opening.
            self.sidebar.keep_visible(scope.clone());
            if !self.sidebar.select(&scope) {
                self.sidebar.set_unread_only(false);
                self.sidebar.set_query("");
                self.sidebar.select(&scope);
            }
        }
        self.sync_room(cx);
        self.set_compact_chat(true);
        self.list_focus.focus(window);
        cx.notify();
    }

    /// Runs from `menu_key` while the overlay is open; true when the key was consumed.
    pub(crate) fn quick_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let m = &event.keystroke.modifiers;
        let plain = !(m.control || m.alt || m.platform);
        if key == "escape" {
            self.close_quick_search(window, cx);
            return true;
        }
        let rows = self.quick_rows().len();
        let seen = self.recents.get(self.recents_account()).seen().len();
        let Some(q) = self.quick.as_mut() else {
            return false;
        };
        if clears(q.zone, key, plain, m.secondary(), !q.term.is_empty(), rows) {
            self.quick_clear(window, cx);
            return true;
        }
        match key {
            "enter" if plain => {
                let (zone, i) = (q.zone, q.active);
                self.quick_pick(zone, i, window, cx);
            }
            "up" | "down" | "tab" | "left" | "right" if plain || key == "tab" => {
                // Left/Right belong to the text field unless the «Visti» row has the highlight.
                if matches!(key, "left" | "right") && q.zone != Zone::Seen {
                    return false;
                }
                if let Some((zone, active)) =
                    step(q.zone, q.active, key, seen, rows, !q.term.is_empty())
                {
                    (q.zone, q.active) = (zone, active);
                    match zone {
                        Zone::Rows => q.scroll.scroll_to_item(active, ScrollStrategy::Top),
                        Zone::Seen => q.seen_scroll.scroll_to_item(active),
                    }
                }
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    fn quick_avatar(
        &self,
        t: matrix_desktop::DesignTokens,
        avatar: Option<&str>,
        name: &str,
        size: f32,
    ) -> gpui::AnyElement {
        match self.avatars.get(avatar) {
            Some(image) => avatar_view::round(image, size),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(size))
                .flex_shrink_0()
                .rounded_full()
                .bg(rgb(matrix_desktop::AVATAR_FALLBACK))
                .text_size(px(size * 0.4))
                .text_color(rgb(t.avatar_text))
                .child(matrix_desktop::initials(name))
                .into_any_element(),
        }
    }

    /// Screen-reader view: the field, the «Visti» row, then the list, name and Matrix ID each.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_quick_search(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<matrix_desktop::accessibility::new_room::Snapshot> {
        use accesskit::Role;
        use matrix_desktop::accessibility::new_room::{Item, Snapshot};
        let q = self.quick.as_ref()?;
        let field = q.field.read(cx);
        let mut items = vec![Item {
            role: Role::TextInput,
            label: field.placeholder.to_owned(),
            checked: None,
            focused: field.focus.is_focused(window),
            bounds: field.semantic_bounds,
        }];
        let item = |label: String, focused: bool| Item {
            role: Role::Button,
            label,
            checked: None,
            focused,
            bounds: None,
        };
        if q.term.is_empty() {
            let recents = self.recents.get(self.recents_account());
            for (i, r) in recents.seen().iter().enumerate() {
                items.push(item(
                    format!("{}, {}", r.name, r.id),
                    q.zone == Zone::Seen && q.active == i,
                ));
            }
        }
        if q.term.is_empty() && !self.quick_rows().is_empty() {
            items.push(item("Svuota".into(), false));
        }
        for (i, hit) in self.quick_rows().iter().enumerate() {
            items.push(item(
                format!("{}, {}", hit.name, hit.secondary),
                q.zone == Zone::Rows && q.active == i,
            ));
        }
        Some(Snapshot { items, join: None })
    }

    pub(crate) fn render_quick_search(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let q = self.quick.as_ref()?;
        q.field.update(cx, |f, _| f.theme = t);
        let typing = !q.term.is_empty();
        let recents = self.recents.get(self.recents_account());
        let rows = self.quick_rows().len();
        let (zone, active) = (q.zone, q.active);
        let header = |label: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(rgb(t.secondary))
                .child(label)
        };
        let seen = (!typing && !recents.seen().is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(header("Visti di recente"))
                .child(
                    div()
                        .id("quick-seen")
                        .flex()
                        .overflow_x_scroll()
                        .track_scroll(&q.seen_scroll)
                        .children(recents.seen().iter().enumerate().map(|(i, r)| {
                            div()
                                .id(("quick-seen-item", i))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .w(px(64.))
                                .flex_shrink_0()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .when(zone == Zone::Seen && active == i, |d| d.bg(rgb(t.input)))
                                .child(self.quick_avatar(t, r.avatar.as_deref(), &r.name, 40.))
                                .child(
                                    div()
                                        .w_full()
                                        .px_1()
                                        .truncate()
                                        .text_center()
                                        .text_size(px(11.))
                                        .text_color(rgb(t.text))
                                        .child(seen_label(&r.name)),
                                )
                                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                    this.quick_highlight(Zone::Seen, i, cx)
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.quick_pick(Zone::Seen, i, window, cx)
                                }))
                        })),
                )
        });
        let searched_header = (!typing && rows > 0).then(|| {
            div()
                .flex()
                .justify_between()
                .child(header("Ricerche recenti"))
                .child(
                    div()
                        .id("quick-clear")
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(t.accent))
                        .child("Svuota")
                        .on_click(cx.listener(|this, _, window, cx| this.quick_clear(window, cx))),
                )
        });
        let list = (rows > 0).then(|| {
            div()
                .relative()
                .h(px((rows as f32 * ROW_H).min(LIST_MAX_H)))
                .child(
                    uniform_list(
                        "quick-rows",
                        rows,
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let hits = this.quick_rows();
                            let (zone, active) = this
                                .quick
                                .as_ref()
                                .map_or((Zone::Rows, 0), |q| (q.zone, q.active));
                            range
                                .filter_map(|i| hits.get(i).map(|hit| (i, hit)))
                                .map(|(i, hit)| {
                                    div()
                                        .id(("quick-row", i))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .h(px(ROW_H))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .when(zone == Zone::Rows && active == i, |d| {
                                            d.bg(rgb(t.input))
                                        })
                                        .child(this.quick_avatar(
                                            t,
                                            hit.avatar.as_deref(),
                                            &hit.name,
                                            28.,
                                        ))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .child(
                                                    div().truncate().text_color(rgb(t.text)).child(
                                                        matrix_desktop::row_text::one_line(
                                                            &hit.name,
                                                        ),
                                                    ),
                                                )
                                                .children(
                                                    Some(secondary_label(&hit.secondary))
                                                        .filter(|s| !s.is_empty())
                                                        .map(|s| {
                                                            div()
                                                                .truncate()
                                                                .text_size(px(12.))
                                                                .text_color(rgb(t.secondary))
                                                                .child(s.to_owned())
                                                        }),
                                                ),
                                        )
                                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                            this.quick_highlight(Zone::Rows, i, cx)
                                        }))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.quick_pick(Zone::Rows, i, window, cx)
                                        }))
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(q.scroll.clone())
                    .h_full(),
                )
                .child(q.scrollbar.render(
                    "quick-scrollbar",
                    scrollbar_view::ScrollTarget::Rooms(q.scroll.clone()),
                    t,
                ))
        });
        Some(
            div()
                .id("quick-search-layer")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_quick_search(window, cx)))
                .child(
                    div()
                        .id("quick-search")
                        .w(px(480.))
                        .max_w_full()
                        .p_3()
                        .gap_2()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(q.field.clone())
                        .children(seen)
                        .children(searched_header)
                        .children(list)
                        .when(typing && rows == 0, |d| {
                            d.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(no_results_color(&t)))
                                    .child("Nessun risultato"),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// The mouse moves the same highlight as the keyboard.
    fn quick_highlight(&mut self, zone: Zone, i: usize, cx: &mut Context<Self>) {
        if let Some(q) = self
            .quick
            .as_mut()
            .filter(|q| (q.zone, q.active) != (zone, i))
        {
            (q.zone, q.active) = (zone, i);
            cx.notify();
        }
    }

    /// «Svuota»: only the search list; «Visti di recente» is history and stays.
    fn quick_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        recents.clear_searched();
        let has_seen = !recents.seen().is_empty();
        self.recents.set(account, recents);
        self.save_recents();
        if let Some(q) = self.quick.as_mut() {
            (q.zone, q.active) = (if has_seen { Zone::Seen } else { Zone::Rows }, 0);
            // The click took the focus off the field: give it back so typing continues.
            q.field.read(cx).focus.focus(window);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Zone::*;

    #[test]
    fn highlight_moves_within_and_between_the_two_zones() {
        // Typing: only the results list exists.
        assert_eq!(step(Rows, 0, "down", 3, 5, true), Some((Rows, 1)));
        assert_eq!(step(Rows, 4, "down", 3, 5, true), Some((Rows, 4)));
        assert_eq!(step(Rows, 0, "up", 3, 5, true), None);
        assert_eq!(step(Rows, 0, "tab", 3, 5, true), None);
        // Empty field: «Visti» row on top of the searches list.
        assert_eq!(step(Seen, 0, "right", 3, 5, false), Some((Seen, 1)));
        assert_eq!(step(Seen, 2, "right", 3, 5, false), Some((Seen, 2)));
        assert_eq!(step(Seen, 0, "left", 3, 5, false), Some((Seen, 0)));
        assert_eq!(step(Seen, 1, "down", 3, 5, false), Some((Rows, 0)));
        assert_eq!(step(Rows, 0, "up", 3, 5, false), Some((Seen, 0)));
        assert_eq!(step(Rows, 2, "up", 3, 5, false), Some((Rows, 1)));
        assert_eq!(step(Seen, 1, "tab", 3, 5, false), Some((Rows, 0)));
        assert_eq!(step(Rows, 1, "tab", 3, 5, false), Some((Seen, 0)));
        // A zone with nothing in it is never entered.
        assert_eq!(step(Seen, 0, "down", 3, 0, false), None);
        assert_eq!(step(Rows, 0, "up", 0, 5, false), None);
        assert_eq!(step(Seen, 0, "tab", 3, 0, false), None);
    }

    #[test]
    fn svuota_from_the_keyboard() {
        // Backspace/Delete on the highlighted searches, empty field.
        assert!(clears(Rows, "backspace", true, false, false, 2));
        assert!(clears(Rows, "delete", true, false, false, 2));
        assert!(!clears(Seen, "backspace", true, false, false, 2));
        // Cmd/Ctrl+Backspace from any zone.
        assert!(clears(Seen, "backspace", false, true, false, 2));
        // Typing: Backspace edits the field. Nothing to clear: no-op.
        assert!(!clears(Rows, "backspace", true, false, true, 2));
        assert!(!clears(Rows, "backspace", true, false, false, 0));
    }

    #[test]
    fn seen_label_is_one_short_line_with_final_ellipsis() {
        assert_eq!(seen_label("Anna"), "Anna");
        assert_eq!(seen_label("12345678"), "12345678");
        assert_eq!(seen_label("017 Progetto lungo"), "017 Pro…");
        assert_eq!(seen_label("a\nb"), "a b");
        assert_eq!(seen_label("àèìòùàèìòù").chars().count(), 8);
    }

    #[test]
    fn raw_room_ids_are_not_shown_but_other_secondary_text_is() {
        assert_eq!(secondary_label("!KBYhOBtiJiD1au:matrix.org"), "");
        assert_eq!(secondary_label("@bob:matrix.org"), "@bob:matrix.org");
        assert_eq!(secondary_label("WhatsApp"), "WhatsApp");
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let l = |c: u32| {
            let v = |s: u32| {
                let x = f64::from((c >> s) & 255) / 255.;
                if x <= 0.03928 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * v(16) + 0.7152 * v(8) + 0.0722 * v(0)
        };
        (l(a).max(l(b)) + 0.05) / (l(a).min(l(b)) + 0.05)
    }

    #[test]
    fn no_results_text_is_readable_in_every_theme() {
        for t in [
            matrix_desktop::LIGHT,
            matrix_desktop::DARK,
            matrix_desktop::DAY,
            matrix_desktop::NIGHT,
        ] {
            let r = contrast(no_results_color(&t), t.surface);
            assert!(r >= 4.5, "{r:.2}");
        }
    }

    #[test]
    fn a_left_room_leaves_both_recent_lists() {
        let item = |id: &str| Recent {
            id: id.into(),
            name: id.into(),
            avatar: None,
        };
        let mut r = matrix_desktop::quick_search::Recents::default();
        (
            r.opened(item("a")),
            r.opened(item("b")),
            r.picked(item("a")),
            r.picked(item("b")),
        );
        r.forget("a");
        assert_eq!(r.seen().len(), 1);
        assert_eq!(r.searched().len(), 1);
        assert_eq!(r.seen()[0].id, "b");
    }
}
