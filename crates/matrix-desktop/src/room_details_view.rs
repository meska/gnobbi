//! Optional read-only sheet over the chat; the default remains two columns.
use crate::switch_row::switch_row;
use crate::*;

/// Shield of an encrypted room: not a theme token, the same green in every theme.
const ENCRYPTED_GREEN: u32 = 0x2e9e5b;

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
                links: {
                    let pending = self.topic_confirm.borrow().capture();
                    self.reconcile_topic_links()
                        .into_iter()
                        .map(|target| matrix_desktop::accessibility::links::Snapshot {
                            requested: pending.as_ref().is_some_and(|p| p.source().instance() == target.instance()),
                            target,
                            bounds: None,
                            focused: false,
                        })
                        .collect()
                },
                confirmation: self.topic_confirm.borrow().capture().map(|request| {
                    matrix_desktop::accessibility::links::ConfirmationSnapshot {
                        request,
                        focused: None,
                        bounds: [None; 3],
                    }
                }),
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
        let focus_panel = matches!(action, DetailsAction::Focus { .. })
            || self.details_zone.contains_focused(window, cx);
        match action {
            DetailsAction::Open(_) => {
                self.set_room_details_open(true);
                self.details_focus = Default::default();
                self.a11y_details_bounds = [None; 6];
            }
            DetailsAction::Close(_) => self.set_room_details_open(false),
            DetailsAction::Focus { control, .. } => self.details_focus = control,
            DetailsAction::Link { instance, focus_only, .. } => {
                self.details_focus = DetailsControl::Topic;
                // Current links only: a node of an older topic text finds no target here.
                let target = self.reconcile_topic_links().into_iter().find(|t| t.instance() == instance);
                if let Some(target) = target
                    && !focus_only
                {
                    self.topic_links.borrow_mut().request(&target, &mut self.topic_confirm.borrow_mut());
                }
            }
            DetailsAction::LinkConfirmation { generation, control, focus_only, .. } => {
                self.reconcile_topic_links();
                let request = self.topic_confirm.borrow().capture().filter(|r| r.generation() == generation);
                if let Some(request) = request
                    && !focus_only
                {
                    let (cancel, confirm) = crate::timeline_view::confirmation_callbacks(
                        self.topic_confirm.clone(),
                        self.topic_links.clone(),
                        request,
                    );
                    match control {
                        matrix_desktop::accessibility::links::ConfirmationControl::Cancel => {
                            cancel();
                        }
                        matrix_desktop::accessibility::links::ConfirmationControl::Open => {
                            if let Some(url) = confirm() {
                                cx.open_url(url.as_str());
                            }
                        }
                    }
                }
            }
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
        if focus_panel && self.room_details_open {
            self.details_zone.focus(window);
        } else {
            self.list_focus.focus(window);
        }
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

    /// The topic a link may come from: only while the panel really shows it.
    fn topic_source(&self) -> Option<(RoomScope, String)> {
        if !self.connected
            || !self.room_details_open
            || self.profile.is_some()
            || self.notifications_open
            || self.verification_open
        {
            return None;
        }
        let room = self.sidebar.selected()?;
        Some((room.scope.clone(), room.info.topic.clone()?))
    }

    /// Re-read the shown topic and drop every link and confirmation that no longer matches it.
    /// Called on each state change and again before any effect, so a stale target never opens.
    pub(crate) fn reconcile_topic_links(&self) -> Vec<matrix_desktop::MessageLinkTarget> {
        let targets = match self.topic_source() {
            Some((scope, text)) => self.topic_links.borrow_mut().topic_targets(&scope, &text),
            None => {
                self.topic_links.borrow_mut().clear();
                Vec::new()
            }
        };
        let stale = self
            .topic_confirm
            .borrow()
            .capture()
            .is_some_and(|r| !self.topic_links.borrow().is_current(r.source()));
        if stale {
            self.topic_confirm.borrow_mut().cancel();
        }
        if !targets.iter().any(|t| self.topic_hover.get() == Some(t.instance())) {
            self.topic_hover.set(None);
        }
        targets
    }

    /// Up/Down/PageUp/PageDown scroll the focused «Argomento» area.
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

    /// The topic text; its links are drawn inline and open only through the confirmation.
    fn topic_text(
        &self,
        topic: &Option<String>,
        targets: &[matrix_desktop::MessageLinkTarget],
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(text) = topic else {
            return div().child("Nessun argomento").into_any_element();
        };
        if targets.is_empty() {
            return div().child(text.clone()).into_any_element();
        }
        let body = gpui::StyledText::new(text.clone()).with_highlights(targets.iter().map(|l| {
            let style = gpui::HighlightStyle {
                color: Some(rgb(t.accent).into()),
                underline: Some(gpui::UnderlineStyle { thickness: px(1.), color: None, wavy: false }),
                ..Default::default()
            };
            (l.range(), style)
        }));
        let (down_layout, up_layout, move_layout) = (body.layout().clone(), body.layout().clone(), body.layout().clone());
        let (down_targets, up_targets, move_targets) = (targets.to_vec(), targets.to_vec(), targets.to_vec());
        let at = |layout: &gpui::TextLayout, targets: &[matrix_desktop::MessageLinkTarget], position| {
            layout
                .index_for_position(position)
                .ok()
                .and_then(|ix| matrix_desktop::link_at(targets, ix).cloned())
        };
        div()
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                this.topic_confirm.borrow_mut().cancel();
                this.topic_pressed.set(at(&down_layout, &down_targets, e.position).map(|l| l.instance()));
                cx.notify();
            }))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(move |this, e: &gpui::MouseUpEvent, _, cx| {
                let pressed = this.topic_pressed.take();
                // Re-read the shown topic first: only a link of the current text, pressed and released on the same spot, asks for confirmation.
                this.reconcile_topic_links();
                if let Some(target) = at(&up_layout, &up_targets, e.position)
                    && pressed == Some(target.instance())
                    && this.topic_links.borrow_mut().request(&target, &mut this.topic_confirm.borrow_mut())
                {
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                let over = at(&move_layout, &move_targets, e.position).map(|l| l.instance());
                if over != this.topic_hover.get() {
                    this.topic_hover.set(over);
                    cx.notify();
                }
            }))
            .child(body)
            .into_any_element()
    }

    pub(crate) fn render_room_details(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = self.theme.tokens();
        // A profile opened from a link has no room behind it: only that sheet is drawn.
        let room = self.sidebar.selected();
        if room.is_none() && self.profile.is_none() {
            return div().into_any_element();
        }
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_control = Some(self.details_focus as usize);
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused_control: Option<usize> = None;
        let topic_links = self.reconcile_topic_links();
        let hide_status = room.is_some_and(|r| self.room_prefs.hide_status(&r.scope));
        let hide_redacted = room.is_some_and(|r| self.room_prefs.hide_redacted(&r.scope).unwrap_or(self.prefs.hide_redacted));
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
                    .when_some(room.filter(|_| self.profile.is_none()), |d, room| d
                    .child(
                    div().relative().flex().flex_col().flex_shrink().min_h_0().child(div().id("room-details-info").flex().flex_col().gap_2().min_h_0()
                    .child(
                        div().flex().items_center().gap_2().flex_shrink_0()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .truncate() // truncate clips, so in a short sheet flex would squash the title to half a line
                                    .child(matrix_desktop::row_text::one_line(&room.name)),
                            )
                            // Count and shield are the visual form of the "N membri" / "Cifratura ..." labels in accessibility/details.rs.
                            .child(div().flex().flex_shrink_0().items_center().gap_1().text_color(rgb(t.secondary)).child(crate::composer_icons::icon(crate::composer_icons::Icon::People, t.secondary)).child(room.info.joined_members.to_string()))
                            .child(crate::composer_icons::icon(
                                crate::composer_icons::Icon::Shield,
                                if room.info.encrypted == Some(true) { ENCRYPTED_GREEN } else { t.error },
                            )),
                    )
                    .children(room.info.topic.as_deref().filter(|s| !s.trim().is_empty()).map(|_| 
                        div().child("Argomento").child(
                            // At most a third of the sheet below the header, then it scrolls (short text: natural height, no thumb).
                            div()
                                .relative()
                                .mt_2()
                                .children({
                                    #[cfg(test)]
                                    { crate::sidebar_navigation_test::observe("room-topic".into()) }
                                    #[cfg(not(test))]
                                    { None::<gpui::AnyElement> }
                                })
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
                                        .when(self.topic_hover.get().is_some(), |d| d.cursor_pointer())
                                        .child(self.topic_text(&room.info.topic, &topic_links, t, cx)),
                                )
                                .child(self.topic_scrollbar.render(
                                    "room-topic-scrollbar",
                                    scrollbar_view::ScrollTarget::Rooms(self.topic_scroll.clone()),
                                    t,
                                )),
                        ),
                    ))
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
                    .child(div().flex().flex_col()
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
                    ))
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
                    )
                    )
                    .child(self.render_people(cx)),
            )
            .when_some(self.topic_confirm.borrow().capture(), |d, request| {
                let (cancel, confirm) = crate::timeline_view::confirmation_callbacks(
                    self.topic_confirm.clone(),
                    self.topic_links.clone(),
                    request.clone(),
                );
                // Same wording and the same two controls as the timeline's confirmation.
                d.child(div().id("topic-link-confirmation").occlude().absolute().bottom_0().left_0().right_0()
                    .px_4().py_3().bg(rgb(t.surface)).border_t_2().border_color(rgb(t.accent))
                    .flex().flex_col().gap_2()
                    .child("Aprire nel browser esterno? Controlla l'indirizzo.")
                    .child(div().text_size(px(12.)).text_color(rgb(t.secondary)).whitespace_normal().child(request.url().as_str().to_owned()))
                    .child(div().flex().justify_end().gap_4()
                        .child(div().id("topic-link-cancel").relative().children({
                                    #[cfg(test)]
                                    { crate::sidebar_navigation_test::observe("topic-link-cancel".into()) }
                                    #[cfg(not(test))]
                                    { None::<gpui::AnyElement> }
                                }).px_4().py_2().border_1().border_color(rgb(t.surface)).cursor_pointer().child("Annulla")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.reconcile_topic_links();
                                cancel();
                                cx.notify();
                            })))
                        .child(div().id("topic-link-open").px_4().py_2().border_1().border_color(rgb(t.accent)).rounded_lg().bg(rgb(t.accent)).text_color(rgb(t.on_accent)).cursor_pointer().child("Apri link")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // Revalidate against the shown topic in this same turn, then open: no await in between.
                                this.reconcile_topic_links();
                                if let Some(url) = confirm() {
                                    cx.open_url(url.as_str());
                                }
                                cx.notify();
                            })))))
            })
            .into_any_element()
    }
}
