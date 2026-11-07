//! Optional read-only sheet over the chat; the default remains two columns.
use crate::switch_row::switch_row;
use crate::*;

impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_details(
        &self,
        window: &Window,
    ) -> matrix_desktop::accessibility::details::DetailsView {
        use matrix_desktop::accessibility::details::{DetailsSnapshot, DetailsView};
        if !self.connected || self.notifications_open || self.verification_open {
            return DetailsView::Unavailable;
        }
        let Some(room) = self.sidebar.selected() else {
            return DetailsView::Unavailable;
        };
        if self.room_details_open {
            DetailsView::Open(Box::new(DetailsSnapshot {
                scope: room.scope.clone(),
                name: room.name.clone(),
                info: room.info.clone(),
                people: self.people_labels(),
                back: self.profile.is_some(),
                acts: self.profile_acts(),
                focus: self.details_focus,
                profile_focus: self.profile_focus,
                bounds: self.a11y_details_bounds,
                hide_status: self.room_prefs.hide_status(&room.scope),
                hide_redacted: self
                    .room_prefs
                    .hide_redacted(&room.scope)
                    .unwrap_or(self.prefs.hide_redacted),
            }))
        } else if window.viewport_size().width >= px(800.) || self.compact_chat {
            DetailsView::Closed(room.scope.clone())
        } else {
            DetailsView::Unavailable
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_details(
        &mut self,
        action: matrix_desktop::accessibility::details::DetailsAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::details::{DetailsAction, DetailsControl};
        if !self.semantic_details(window).allows(&action) {
            return;
        }
        match action {
            DetailsAction::Open(_) => {
                self.set_room_details_open(true);
                self.details_focus = Default::default();
                self.a11y_details_bounds = [None; 6];
            }
            DetailsAction::Close(_) => self.set_room_details_open(false),
            DetailsAction::Focus { control, .. } => self.details_focus = control,
            DetailsAction::FocusRing { index, .. } => self.profile_focus = index,
            DetailsAction::Act { act, .. } => self.profile_act(act, window, cx),
            DetailsAction::Profile { user_id, .. } => self.open_profile(user_id, cx),
            DetailsAction::Members(_) => self.profile = None,
            DetailsAction::HideStatus(_) => {
                self.details_focus = DetailsControl::HideStatus;
                self.toggle_hide_status(cx);
            }
            DetailsAction::HideRedacted(_) => {
                self.details_focus = DetailsControl::HideRedacted;
                self.toggle_room_hide_redacted(cx);
            }
            DetailsAction::Edit(_) => {
                self.details_focus = DetailsControl::Edit;
                // The edit form takes the focus: skip the list_focus below.
                self.open_room_edit(window, cx);
                cx.notify();
                return;
            }
            DetailsAction::Copy(scope) => {
                self.details_focus = DetailsControl::Copy;
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                    scope.room_id.as_str().to_owned(),
                ));
            }
        }
        self.list_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn toggle_hide_status(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.sidebar.selected().map(|r| r.scope.clone()) else {
            return;
        };
        self.room_prefs.toggle_hide_status(&scope);
        if let Some(dirs) = &self.prefs_dirs
            && let Err(error) = self.room_prefs.save(dirs)
        {
            eprintln!("Gnobbi: cannot save room preferences: {error}");
        }
        cx.notify();
    }

    pub(crate) fn toggle_room_hide_redacted(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.sidebar.selected().map(|r| r.scope.clone()) else {
            return;
        };
        let now = self.room_prefs.hide_redacted(&scope).unwrap_or(self.prefs.hide_redacted);
        self.room_prefs.set_hide_redacted(&scope, !now);
        if let Some(dirs) = &self.prefs_dirs
            && let Err(error) = self.room_prefs.save(dirs)
        {
            eprintln!("Gnobbi: cannot save room preferences: {error}");
        }
        cx.notify();
    }

    /// Up/Down/PageUp/PageDown scroll the focused «Argomento» area.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn topic_scroll_key(&self, key: &str) -> bool {
        let handle = self.topic_scroll.0.borrow().base_handle.clone();
        let page = f32::from(handle.bounds().size.height);
        let delta = match key {
            "up" => 20.,
            "down" => -20.,
            "pageup" => page,
            "pagedown" => -page,
            _ => return false,
        };
        let max = f32::from(handle.max_offset().height);
        let y = (f32::from(handle.offset().y) + delta).clamp(-max, 0.);
        handle.set_offset(point(px(0.), px(y)));
        true
    }

    fn details_observer(&self, _index: usize, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let entity = _cx.entity();
            Some(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let scale = f64::from(window.scale_factor());
                        let rect = accesskit::Rect::new(
                            f64::from(f32::from(bounds.left())) * scale,
                            f64::from(f32::from(bounds.top())) * scale,
                            f64::from(f32::from(bounds.right())) * scale,
                            f64::from(f32::from(bounds.bottom())) * scale,
                        );
                        if entity.read(cx).a11y_details_bounds[_index] != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.a11y_details_bounds[_index] = Some(rect);
                                    cx.notify();
                                })
                            });
                        }
                    },
                )
                .absolute()
                .inset_0()
                .into_any_element(),
            )
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }

    pub(crate) fn render_spaces(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            self.render_accessible_spaces(_window, cx)
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            let t = self.theme.tokens();
            let active = self.sidebar.active_space();
            let label = self
                .sidebar
                .spaces()
                .find(|r| Some(&r.scope) == active)
                .map_or("Tutte", |r| r.name.as_str());
            let choices = std::iter::once((None, "Tutte le stanze".to_owned())).chain(
                self.sidebar
                    .spaces()
                    .map(|r| (Some(r.scope.clone()), r.name.clone())),
            );
            let toggle_visit = self.sidebar.navigation_callback();
            div()
                .track_focus(&self.spaces_ui.focus)
                .flex()
                .flex_col()
                .flex_shrink_0()
                .text_size(px(12.))
                .child(
                    div()
                        .id("spaces-toggle")
                        .relative()
                        .children({
                            #[cfg(test)]
                            {
                                crate::sidebar_navigation_test::observe("compact-space0".into())
                            }
                            #[cfg(not(test))]
                            {
                                None::<gpui::AnyElement>
                            }
                        })
                        .border_1()
                        .border_color(rgb(
                            if self.spaces_ui.focus.is_focused(_window)
                                && self.spaces_ui.target.is_none()
                            {
                                t.accent
                            } else {
                                t.surface
                            },
                        ))
                        .h(px(32.))
                        .px_4()
                        .flex()
                        .items_center()
                        .text_color(rgb(t.accent))
                        .cursor_pointer()
                        .overflow_hidden()
                        .child(format!("Spazi: {label}"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigation_wide = window.viewport_size().width >= px(800.);
                            this.refresh_navigation_visibility();
                            if !toggle_visit(&this.sidebar) {
                                return;
                            }
                            this.set_spaces_open(!this.spaces_open);
                            this.list_focus.focus(window);
                            cx.notify();
                        })),
                )
                .when(self.spaces_open, |d| {
                    d.child(
                        div()
                            .id("space-choices")
                            .max_h(px(192.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(choices.enumerate().map(|(index, (scope, name))| {
                                let selected = scope.as_ref() == active;
                                let source_visit = self.sidebar.navigation_callback();
                                div()
                                    .id(("space-choice", index))
                                    .relative()
                                    .children({
                                        #[cfg(test)]
                                        {
                                            crate::sidebar_navigation_test::observe(format!(
                                                "compact-space{}",
                                                index + 1
                                            ))
                                        }
                                        #[cfg(not(test))]
                                        {
                                            None::<gpui::AnyElement>
                                        }
                                    })
                                    .h(px(32.))
                                    .flex_shrink_0()
                                    .px_4()
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .overflow_hidden()
                                    .bg(rgb(if selected { t.selected } else { t.input }))
                                    .border_1()
                                    .border_color(rgb(
                                        if self.spaces_ui.focus.is_focused(_window)
                                            && self.spaces_ui.target == Some(scope.clone())
                                        {
                                            t.accent
                                        } else {
                                            t.border
                                        },
                                    ))
                                    .child(name)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigation_wide =
                                            window.viewport_size().width >= px(800.);
                                        this.refresh_navigation_visibility();
                                        if !source_visit(&this.sidebar)
                                            || !this.sidebar.set_space(scope.clone())
                                        {
                                            return;
                                        }
                                        this.set_spaces_open(false);
                                        this.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
                                        this.list_focus.focus(window);
                                        cx.notify();
                                    }))
                            })),
                    )
                })
                .into_any_element()
        }
    }

    pub(crate) fn render_room_details(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = self.theme.tokens();
        let Some(room) = self.sidebar.selected() else {
            return div().into_any_element();
        };
        let encryption = match room.info.encrypted {
            Some(true) => "Cifratura E2EE attiva",
            Some(false) => "Cifratura E2EE non attiva",
            None => "Cifratura non confermata dallo stato SDK",
        };
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_control = Some(self.details_focus as usize);
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused_control: Option<usize> = None;
        let hide_status = self.room_prefs.hide_status(&room.scope);
        let hide_redacted = self.room_prefs.hide_redacted(&room.scope).unwrap_or(self.prefs.hide_redacted);
        div()
            .id("room-details")
            .children({
                #[cfg(test)]
                {
                    crate::sidebar_navigation_test::observe("details-panel".into())
                }
                #[cfg(not(test))]
                {
                    None::<gpui::AnyElement>
                }
            })
            .track_focus(&self.details_zone)
            // Own the wheel: without it the hit-test lets events through to the timeline list.
            .occlude()
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(window.viewport_size().width.min(px(360.)))
            .flex()
            .flex_col()
            .bg(rgb(t.surface))
            .border_l_1()
            .border_color(rgb(t.border))
            .when(self.profile.is_none(), |d| d.child(
                div()
                    .h(px(t.header))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Informazioni stanza"),
                    )
                    .child(
                        div()
                            .id("room-details-close")
                            .relative()
                            .children(self.details_observer(0, cx))
                            .children({
                                #[cfg(test)]
                                {
                                    crate::sidebar_navigation_test::observe("details-close".into())
                                }
                                #[cfg(not(test))]
                                {
                                    None::<gpui::AnyElement>
                                }
                            })
                            .when(focused_control == Some(0), |d| {
                                d.border_1().border_color(rgb(t.accent))
                            })
                            .w(px(32.)).h(px(32.)).flex().items_center().justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(t.border)))
                            .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_room_details_open(false);
                                if this.details_zone.contains_focused(window, cx) {
                                    this.list_focus.focus(window);
                                }
                                cx.notify();
                            })),
                    ),
            ))
            .child(
                div()
                    .id("room-details-content")
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    // An opened profile takes the whole sheet.
                    .when(self.profile.is_none(), |d| d
                    .child(
                    div().relative().flex().flex_col().flex_shrink().min_h_0().child(div().id("room-details-info").flex().flex_col().gap_4().min_h_0().track_scroll(&self.info_scroll.0.borrow().base_handle).overflow_y_scroll()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .flex_shrink_0() // truncate clips, so in a short sheet flex would squash the title to half a line
                            .truncate()
                            .child(matrix_desktop::row_text::one_line(&room.name)),
                    )
                    .child(format!(
                        "{} {}",
                        room.info.joined_members,
                        if room.info.joined_members == 1 {
                            "membro"
                        } else {
                            "membri"
                        }
                    ))
                    .child(
                        div().child("Argomento").child(
                            // At most a third of the sheet below the header, then it scrolls (short text: natural height, no thumb).
                            div()
                                .relative()
                                .mt_2()
                                .children(self.details_observer(1, cx))
                                .when(focused_control == Some(1), |d| {
                                    d.border_1().border_color(rgb(t.accent))
                                })
                                .child(
                                    div()
                                        .id("room-topic")
                                        .max_h((window.viewport_size().height - px(t.header)) / 3.)
                                        .overflow_y_scroll()
                                        .track_scroll(&self.topic_scroll.0.borrow().base_handle)
                                        .child(
                                            room.info
                                                .topic
                                                .clone()
                                                .unwrap_or_else(|| "Nessun argomento".into()),
                                        ),
                                )
                                .child(self.topic_scrollbar.render(
                                    "room-topic-scrollbar",
                                    scrollbar_view::ScrollTarget::Rooms(self.topic_scroll.clone()),
                                    t,
                                )),
                        ),
                    )
                    .child(encryption)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(t.secondary))
                            .child("ID stanza")
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .truncate()
                                            .child(room.scope.room_id.as_str().to_owned()),
                                    )
                                    // Icon only, same 36x36 box and focus ring as the panel opener; label for screen readers lives in the a11y tree.
                                    .child(
                                        div()
                                            .id("copy-room-id")
                                            .relative()
                                            .children(self.details_observer(2, cx))
                                            .flex_shrink_0()
                                            .w(px(36.))
                                            .h(px(36.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_lg()
                                            .border_1()
                                            .border_color(if input_modality::only_keys(focused_control == Some(2), cx) {
                                                rgb(t.accent)
                                            } else {
                                                gpui::rgba(0)
                                            })
                                            .hover(|s| s.bg(rgb(t.border)))
                                            .cursor_pointer()
                                            .child(composer_icons::icon(composer_icons::Icon::Copy, t.accent))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Some(room) = this.sidebar.selected() {
                                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                        room.scope.room_id.as_str().to_owned(),
                                                    ));
                                                }
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        switch_row(
                            "room-hide-status", "Messaggi di stato", !hide_status, true, input_modality::only_keys(focused_control == Some(3), cx), t,
                        )
                        .relative()
                        .children(self.details_observer(3, cx))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_hide_status(cx))),
                    )
                    .child(
                        switch_row(
                            "room-hide-redacted", "Messaggi eliminati", !hide_redacted, true, input_modality::only_keys(focused_control == Some(4), cx), t,
                        )
                        .relative()
                        .children(self.details_observer(4, cx))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_room_hide_redacted(cx))),
                    )
                    .child(
                        div()
                            .id("room-edit-open")
                            .relative()
                            .children(self.details_observer(5, cx))
                            .when(focused_control == Some(5), |d| d.border_1().border_color(rgb(t.accent)))
                            .cursor_pointer()
                            .text_color(rgb(t.accent))
                            .child("Modifica")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_room_edit(window, cx)
                            })),
                    ))
                    .child(self.info_scrollbar.render(
                        "room-info-scrollbar",
                        scrollbar_view::ScrollTarget::Rooms(self.info_scroll.clone()),
                        t,
                    )))
                    )
                    .child(self.render_people(cx)),
            )
            .into_any_element()
    }
}
