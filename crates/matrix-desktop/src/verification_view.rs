//! Original device verification screen; no IO or trust decisions during rendering.
use crate::*;

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) struct DeviceScroll {
    handle: gpui::ScrollHandle,
    anchors: [gpui::ScrollAnchor; 2],
    rows:
        std::collections::HashMap<accesskit::NodeId, (gpui::ScrollAnchor, Option<accesskit::Rect>)>,
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
impl Default for DeviceScroll {
    fn default() -> Self {
        let handle = gpui::ScrollHandle::new();
        Self {
            anchors: std::array::from_fn(|_| gpui::ScrollAnchor::for_handle(handle.clone())),
            handle,
            rows: Default::default(),
        }
    }
}

/// Emoji cell width and glyph line height (logical px): the line is taller than the glyph so it is never cut.
pub(crate) const EMOJI_CELL: f32 = 72.;
pub(crate) const EMOJI_LINE: f32 = 52.;

impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_devices(
        &self,
        window: &Window,
    ) -> matrix_desktop::accessibility::devices::DeviceView {
        use matrix_desktop::accessibility::devices::{DeviceOverview, DeviceView};
        let popup = self.verification.popup_open();
        if !self.connected || (!popup && (self.notifications_open || self.room_details_open)) {
            DeviceView::Unavailable
        } else if popup || self.verification_open {
            let panel = DeviceOverview::project(
                self.active_scope.account_id,
                &self.verification,
                self.device_focus,
                self.a11y_device_bounds,
            )
            .with_devices(self.semantic_device_rows());
            DeviceView::Open(Box::new(
                self.a11y_device_nodes.borrow_mut().scope_panel(panel),
            ))
        } else if window.viewport_size().width >= px(800.) || self.compact_chat {
            DeviceView::Closed(self.active_scope.account_id)
        } else {
            DeviceView::Unavailable
        }
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn semantic_device_rows(&self) -> Vec<matrix_desktop::accessibility::devices::SemanticDevice> {
        let mut rows = self
            .a11y_device_nodes
            .borrow_mut()
            .project(self.active_scope.account_id, &self.verification);
        for row in &mut rows {
            row.bounds = self.device_scroll.rows.get(&row.id).and_then(|r| r.1);
        }
        rows
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_devices(
        &mut self,
        action: matrix_desktop::accessibility::devices::DeviceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::devices::{DeviceAction, DeviceControl};
        if !self.semantic_devices(window).allows(&action) {
            return;
        }
        match action {
            DeviceAction::OpenDelete { device, .. } => {
                if let Some(index) = self.verification.devices().iter().position(|d| *d == device) {
                    self.open_device_delete(index, window, cx);
                    self.device_focus = DeviceControl::DialogField;
                }
                return;
            }
            DeviceAction::OpenInactive(_) => {
                self.open_inactive_delete(window, cx);
                self.device_focus = DeviceControl::DialogField;
                return;
            }
            DeviceAction::CycleThreshold(_) => {
                self.verification.cycle_delete_threshold(matrix_desktop::time_fmt::now_ms());
                cx.notify();
                return;
            }
            DeviceAction::OpenRename(_) => {
                self.open_device_rename(window, cx);
                self.device_focus = DeviceControl::DialogField;
                return;
            }
            DeviceAction::CancelDelete(_) => {
                let renaming = self.verification.rename_dialog().is_some();
                let inactive = self.verification.delete_dialog().is_some_and(|d| d.threshold.is_some());
                let back = self.verification.delete_dialog().and_then(|dialog| {
                    self.semantic_device_rows().into_iter().find(|r| dialog.threshold.is_none() && dialog.device_ids.contains(&r.device.device_id))
                });
                self.close_device_delete(window, cx);
                self.device_focus = if renaming { DeviceControl::Rename } else { back.map_or(if inactive { DeviceControl::Inactive } else { DeviceControl::Close }, |r| DeviceControl::Delete(r.id)) };
                return;
            }
            DeviceAction::SubmitDelete(_) => {
                self.submit_device_delete(cx);
                return;
            }
            DeviceAction::Open(_) => {
                self.set_verification_open(true);
                self.device_focus = Default::default();
                self.a11y_device_bounds = [None; 2];
                self.verification.refresh();
            }
            DeviceAction::Popup { button, .. } => {
                self.verification.popup_press(button);
                cx.notify();
                return;
            }
            DeviceAction::Close(_) => self.set_verification_open(false),
            DeviceAction::Focus { control, .. } => {
                if let DeviceControl::Popup(button) = control {
                    self.verification.popup_set_focus(button);
                    cx.notify();
                    return;
                }
                self.device_focus = control;
                if control == DeviceControl::DialogField {
                    if let Some(field) = &self.delete_field {
                        field.read(cx).focus.focus(window);
                    }
                    return;
                }
                if control != DeviceControl::Close {
                    let anchor = match control {
                        DeviceControl::Device(id) | DeviceControl::Delete(id) => {
                            self.device_scroll.rows.get(&id).map(|r| r.0.clone())
                        }
                        DeviceControl::Rename | DeviceControl::Inactive | DeviceControl::DialogThreshold | DeviceControl::DialogCancel | DeviceControl::DialogConfirm => None,
                        DeviceControl::Popup(_) => None,
                        _ => Some(self.device_scroll.anchors[control.index().unwrap()].clone()),
                    };
                    if let Some(anchor) = anchor {
                        anchor.scroll_to(window, cx);
                    }
                }
            }
            DeviceAction::Refresh(_) => {
                self.device_focus = DeviceControl::Refresh;
                self.verification.refresh();
            }
            DeviceAction::Begin { device, .. } => {
                if let Some(index) = self
                    .verification
                    .devices()
                    .iter()
                    .position(|d| *d == device)
                {
                    self.verification.begin(index);
                    self.device_focus = DeviceControl::Close;
                }
            }
        }
        self.list_focus.focus(window);
        cx.notify();
    }

    fn device_anchor(&self, _index: usize) -> Option<gpui::ScrollAnchor> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            if _index < 2 {
                Some(self.device_scroll.anchors[_index].clone())
            } else {
                self.device_scroll
                    .rows
                    .get(&accesskit::NodeId(_index as u64))
                    .map(|r| r.0.clone())
            }
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }

    fn device_scroll_handle(&self) -> Option<gpui::ScrollHandle> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            Some(self.device_scroll.handle.clone())
        }
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }

    fn device_observer(&self, _index: usize, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
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
                        let previous = if _index < 2 {
                            entity.read(cx).a11y_device_bounds[_index]
                        } else {
                            entity
                                .read(cx)
                                .device_scroll
                                .rows
                                .get(&accesskit::NodeId(_index as u64))
                                .and_then(|r| r.1)
                        };
                        if previous != Some(rect) {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    if _index < 2 {
                                        this.a11y_device_bounds[_index] = Some(rect);
                                    } else if let Some(row) = this
                                        .device_scroll
                                        .rows
                                        .get_mut(&accesskit::NodeId(_index as u64))
                                    {
                                        row.1 = Some(rect);
                                    }
                                    cx.notify();
                                });
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

    fn open_device_delete(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.verification.open_delete(index) {
            self.start_delete_field(window, cx);
        }
    }

    /// «Disconnetti inattivi…»: the same dialog and password field, in its multi-device form.
    fn open_inactive_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.verification.open_delete_inactive(matrix_desktop::time_fmt::now_ms()) {
            self.start_delete_field(window, cx);
        }
    }

    fn start_delete_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(true, cx);
            field.enabled = true;
            field.placeholder = "Password";
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field.read(cx).focus.focus(window);
        self.delete_field = Some(field);
        cx.notify();
    }

    /// «Rinomina» reuses the Disconnetti dialog and its field, as a plain (not password) text field.
    pub(crate) fn open_device_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.verification.open_rename() {
            return;
        }
        let name = self.verification.own().map(|d| matrix_desktop::device_title(d).to_owned()).unwrap_or_default();
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field.placeholder = "Nome";
            field.input.replace(None, &name);
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field.read(cx).focus.focus(window);
        self.delete_field = Some(field);
        cx.notify();
    }

    fn close_device_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.verification.cancel_delete();
        self.verification.cancel_rename();
        self.delete_field = None;
        self.list_focus.focus(window);
        cx.notify();
    }

    fn submit_device_delete(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.delete_field.clone() else { return };
        if self.verification.rename_dialog().is_some() {
            let name = field.read(cx).input.display_text();
            self.verification.submit_rename(&name);
            cx.notify();
            return;
        }
        // A refused submit (empty field, request in flight) leaves the text alone.
        if !self.verification.can_submit_delete(!field.read(cx).input.display_text().is_empty()) {
            return;
        }
        // The field is emptied right here; the password only travels inside the command.
        let password = field.update(cx, |f, _| f.input.take());
        self.verification.submit_delete(password);
        cx.notify();
    }

    fn render_device_delete(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (title, error, busy, confirm) = match (self.verification.delete_dialog(), self.verification.rename_dialog()) {
            (Some(d), _) => (d.name.clone(), d.error, d.busy, "Disconnetti"),
            (None, Some(d)) => ("Rinomina".to_owned(), d.error, d.busy, "Salva"),
            _ => return None,
        };
        let field = self.delete_field.clone()?;
        field.update(cx, |f, _| f.theme = t);
        let refocus = self.verification.delete_dialog().is_some_and(|d| d.take_refocus());
        let filled = !field.read(cx).input.display_text().is_empty();
        let confirm_color = if busy || (confirm == "Disconnetti" && !filled) { t.disabled } else if confirm == "Salva" { t.accent } else { t.error };
        let focus_field = field.read(cx).focus.clone();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focus = self.device_focus;
        let button = |id: &'static str, label: &'static str, color: u32| {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            let on = (id == "device-delete-cancel" && focus == matrix_desktop::accessibility::devices::DeviceControl::DialogCancel)
                || (id == "device-delete-confirm" && focus == matrix_desktop::accessibility::devices::DeviceControl::DialogConfirm);
            #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
            let on = false;
            div().id(id).px_3().py_2().rounded_md().cursor_pointer().text_color(rgb(color)).when(on, |d| d.border_1().border_color(rgb(t.accent))).child(label)
        };
        let inactive = self.verification.delete_dialog().and_then(|d| Some((d.threshold?.label(), d.counter(), d.unknown_note(), d.device_ids.is_empty())));
        let targets: Vec<String> = self.verification.delete_targets().into_iter()
            .map(|x| matrix_desktop::row_text::one_line(&format!("{} · {}", matrix_desktop::device_title(x), x.last_seen_ts.map_or(String::new(), matrix_desktop::time_fmt::day_or_time))).to_string())
            .collect();
        let dialog = div().id("device-delete-dialog")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.submit_device_delete(cx),
                    "escape" => this.close_device_delete(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.text)).child(matrix_desktop::row_text::one_line(&title)))
            .when_some(inactive, |d, (threshold, counter, unknown, _)| d
                .child(div().id("device-delete-threshold").cursor_pointer().text_color(rgb(t.accent)).py_1()
                    .child(format!("Soglia inattività: {threshold}"))
                    .on_click(cx.listener(|this, _, _, cx| { this.verification.cycle_delete_threshold(matrix_desktop::time_fmt::now_ms()); cx.notify(); })))
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(counter))
                .child(div().id("device-delete-list").max_h(px(140.)).overflow_y_scroll().text_size(px(12.)).flex().flex_col()
                    .children(targets.into_iter().map(|line| div().child(line))))
                .when_some(unknown, |d, u| d.child(div().text_size(px(12.)).text_color(rgb(t.secondary)).child(u))))
            .child(field)
            .when(refocus, |d| d.child(gpui::canvas(|_, _, _| (), move |_, _, window, _| focus_field.focus(window)).size_0()))
            .when_some(error, |d, e| d.child(div().text_size(px(13.)).text_color(rgb(t.error)).child(e)))
            .child(div().flex().justify_end().gap_1()
                .child(button("device-delete-cancel", "Annulla", t.accent).on_click(cx.listener(|this, _, window, cx| this.close_device_delete(window, cx))))
                .child(button("device-delete-confirm", confirm, confirm_color)
                    .on_click(cx.listener(|this, _, _, cx| this.submit_device_delete(cx)))));
        Some(crate::menu_view::dialog_shell("device-delete-layer", dialog, 340., t, cx.listener(|this, _, window, cx| this.close_device_delete(window, cx))).into_any_element())
    }

    /// Popup keys: the popup is modal, so every key stops here (Esc closes, Tab cycles, Enter/Space press).
    pub(crate) fn popup_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.verification.popup_close(),
            "tab" => self.verification.popup_tab(event.keystroke.modifiers.shift),
            "enter" | "space" => {
                if let Some(button) = self.verification.popup_focused() {
                    self.verification.popup_press(button);
                }
            }
            _ => (),
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// The verification popup, drawn once by `Shell::render` above every screen.
    pub(crate) fn render_verification_popup(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use matrix_desktop::{PopupBody, PopupButton};
        let view = self.verification.popup_view()?;
        let t = self.theme.tokens();
        let focused = self.verification.popup_focused();
        let ring = move |b: PopupButton| if focused == Some(b) { rgb(t.accent) } else { gpui::rgba(0) };
        let button = |b: PopupButton| {
            div().id(("verification-popup-button", b.index())).px_4().py_2().rounded_md().cursor_pointer().bg(rgb(t.input))
                .border_1().border_color(ring(b))
                .text_color(rgb(if matches!(b, PopupButton::NoMatch | PopupButton::Decline) { t.error } else { t.accent }))
                .child(b.label())
                .on_click(cx.listener(move |this, _, _, cx| { this.verification.popup_press(b); cx.notify(); }))
        };
        // No fixed height anywhere: the cells grow with the glyph and wrap onto a second row at large scales.
        let body = match view.body {
            PopupBody::Waiting => Some(div().text_color(rgb(t.secondary)).child("In attesa dell’altro dispositivo").into_any_element()),
            PopupBody::Empty => None,
            PopupBody::Request(who) => Some(div().child(matrix_desktop::row_text::one_line(&who)).into_any_element()),
            PopupBody::Outcome(text) => Some(div().child(text).into_any_element()),
            PopupBody::Decimals((a, b, c)) => Some(div().id("verification-comparison").text_size(px(28.)).font_weight(gpui::FontWeight::SEMIBOLD).child(format!("{a}   {b}   {c}")).into_any_element()),
            PopupBody::Emoji(emoji) => Some(div().id("verification-emoji").flex().flex_wrap().justify_center().gap_0p5().children(
                emoji.into_iter().enumerate().map(|(i, (symbol, name))| {
                    div().id(("verification-emoji-cell", i)).flex().flex_col().items_center().flex_shrink_0().w(px(EMOJI_CELL))
                        .children({
                            #[cfg(test)] { crate::verification_popup_test::observe(i) }
                            #[cfg(not(test))] { None::<gpui::AnyElement> }
                        })
                        .child(div().text_size(px(40.)).line_height(px(EMOJI_LINE)).child(symbol))
                        .child(div().w_full().truncate().text_size(px(12.)).text_center().text_color(rgb(t.secondary)).child(name))
                }),
            ).into_any_element()),
        };
        let close = div().id("verification-popup-close").w(px(32.)).h(px(32.)).flex().items_center().justify_center().cursor_pointer()
            .border_1().border_color(ring(PopupButton::Close)).hover(|s| s.bg(rgb(t.border)))
            .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
            .on_click(cx.listener(|this, _, _, cx| { this.verification.popup_close(); cx.notify(); }));
        let dialog = div().id("verification-popup").font_family(matrix_desktop::ui_font()).text_size(px(14.)).text_color(rgb(t.text))
            .children({
                #[cfg(test)] { crate::verification_popup_test::observe_dialog() }
                #[cfg(not(test))] { None::<gpui::AnyElement> }
            })
            .child(div().flex().items_center().justify_between().gap_2()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_size(px(16.)).text_color(rgb(t.text)).child(view.title))
                .child(close))
            .children(body)
            .child(div().flex().flex_wrap().justify_center().gap_2().pt_2()
                .children(view.buttons.into_iter().map(button)));
        Some(crate::menu_view::dialog_shell("verification-popup-layer", dialog, 560., t, cx.listener(|this, _, _, cx| { this.verification.popup_close(); cx.notify(); })).into_any_element())
    }

    pub(crate) fn render_verification(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme.tokens();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let semantic_rows = self.semantic_device_rows();
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let semantic_row_ids: Vec<usize> = semantic_rows
            .iter()
            .map(|row| {
                self.device_scroll.rows.entry(row.id).or_insert_with(|| {
                    (
                        gpui::ScrollAnchor::for_handle(self.device_scroll.handle.clone()),
                        None,
                    )
                });
                usize::try_from(row.id.0).expect("Linux semantic device ID must fit usize")
            })
            .collect();
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let semantic_row_ids: Vec<usize> = vec![];
        let panel = &self.verification;
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused_control = matrix_desktop::accessibility::devices::DeviceOverview::project(
            self.active_scope.account_id,
            panel,
            self.device_focus,
            self.a11y_device_bounds,
        )
        .with_devices(semantic_rows)
        .focus
        .index();
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused_control: Option<usize> = None;
        let delete_dialog = self.render_device_delete(t, cx);
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let inactive_focused = self.device_focus == matrix_desktop::accessibility::devices::DeviceControl::Inactive;
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let inactive_focused = false;
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let rename_focused = self.device_focus == matrix_desktop::accessibility::devices::DeviceControl::Rename;
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let rename_focused = false;
        let rename = panel.own().map(|_| div().id("verification-device-rename").cursor_pointer().flex_shrink_0().text_color(rgb(t.accent))
            .when(rename_focused, |d| d.border_1().border_color(rgb(t.accent)))
            .child("Rinomina")
            .on_click(cx.listener(|this, _, window, cx| this.open_device_rename(window, cx))));
        div().size_full().relative().flex().flex_col().bg(rgb(t.surface)).text_color(rgb(t.text))
            .font_family(matrix_desktop::UI_FONT).text_size(px(14.))
            .track_focus(&self.list_focus)
            .capture_key_down(cx.listener(Self::navigation_key))
            .child(div().h(px(64.)).flex_shrink_0().px_4().flex().items_center().justify_between().border_b_1().border_color(rgb(t.border))
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Verifica dispositivi"))
                .child(div().id("verification-close").relative().children(self.device_observer(0, cx))
                    .when(focused_control == Some(0), |d| d.border_1().border_color(rgb(t.accent)))
                    .w(px(32.)).h(px(32.)).flex().items_center().justify_center().cursor_pointer()
                    .hover(|s| s.bg(rgb(t.border)))
                    .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.accent))
                    .on_click(cx.listener(|this, _, _, cx| { this.set_verification_open(false); cx.notify(); }))))
            .child(div().id("verification-content").flex_1().min_h_0().overflow_y_scroll()
                .when_some(self.device_scroll_handle(), |d, handle| d.track_scroll(&handle))
                .p_4().flex().flex_col().gap_3()
                .when_some(panel.own_device(), |d, id| d.child(div().w_full().flex_shrink_0().min_h(px(56.)).px_3().py_2().rounded_lg().bg(rgb(t.input)).flex().flex_col().gap_1()
                    .child(div().flex().justify_between().gap_2()
                        .child(div().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD).child(match panel.own() {
                            Some(own) if own.display_name.is_some() => format!("Questo dispositivo · {}", matrix_desktop::device_title(own)),
                            _ => "Questo dispositivo".to_owned(),
                        }))
                        .children(rename))
                    .child(div().min_w_0().text_size(px(12.)).child(matrix_desktop::row_text::one_line(&match panel.own() {
                        Some(own) => matrix_desktop::device_detail(own, matrix_desktop::time_fmt::day_or_time),
                        None => id.to_owned(),
                    })))))
                .child("Altri dispositivi")
                .child(div().flex().gap_4().child(div().id("verification-refresh").anchor_scroll(self.device_anchor(1)).relative().children(self.device_observer(1, cx))
                    .when(focused_control == Some(1), |d| d.border_1().border_color(rgb(t.accent)))
                    .cursor_pointer().text_color(rgb(t.accent)).child("Aggiorna dispositivi")
                    .on_click(cx.listener(|this, _, _, cx| { this.verification.refresh(); cx.notify(); })))
                    .when(panel.has_inactive(matrix_desktop::time_fmt::now_ms()), |d| d.child(div().id("verification-inactive").cursor_pointer().text_color(rgb(t.error))
                        .when(inactive_focused, |d| d.border_1().border_color(rgb(t.accent)))
                        .child("Disconnetti inattivi…")
                        .on_click(cx.listener(|this, _, window, cx| this.open_inactive_delete(window, cx))))))
                .when(panel.devices().is_empty(), |d| d.child("Nessun altro dispositivo disponibile"))
                .children(panel.devices().iter().enumerate().map(|(index, device)| {
                    let row_index = semantic_row_ids.get(index).copied();
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    let row_focused = row_index.is_some_and(|id| self.device_focus == matrix_desktop::accessibility::devices::DeviceControl::Device(accesskit::NodeId(id as u64))) && panel.can_begin();
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    let row_focused = false;
                    // Built before the row: the row's source slice ends at its own click handler.
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    let delete_focused = row_index.is_some_and(|id| self.device_focus == matrix_desktop::accessibility::devices::DeviceControl::Delete(accesskit::NodeId(id as u64)));
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    let delete_focused = false;
                    let disconnect = div().id(("verification-device-delete", index)).cursor_pointer().text_color(rgb(t.error))
                        .when(delete_focused, |d| d.border_1().border_color(rgb(t.accent)))
                        .child("Disconnetti")
                        .on_click(cx.listener(move |this, _, window, cx| { this.open_device_delete(index, window, cx); cx.stop_propagation(); }));
                    div().id(("verification-device", index)).relative()
                        .anchor_scroll(row_index.and_then(|id| self.device_anchor(id)))
                        .children(row_index.and_then(|id| self.device_observer(id, cx)))
                        .when(row_focused, |d| d.border_1().border_color(rgb(t.accent)))
                        // flex_shrink_0 + no overflow clip on the row: the scroll area scrolls, rows keep their content height.
                        .w_full().flex_shrink_0().min_h(px(56.)).px_3().py_2().rounded_lg().bg(rgb(t.input))
                        .flex().flex_col().gap_1()
                        .child(div().flex().justify_between().gap_2()
                            .child(div().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD).child(matrix_desktop::row_text::one_line(matrix_desktop::device_title(device))))
                            .child(div().flex().gap_3().flex_shrink_0()
                                .when(!device.verified, |d| d.child(div().text_color(rgb(t.accent)).child("Verifica")))
                                .child(disconnect)))
                        .child(div().min_w_0().text_size(px(12.)).text_color(rgb(if device.verified { t.secondary } else { t.error }))
                            .child(matrix_desktop::row_text::one_line(&matrix_desktop::device_detail(device, matrix_desktop::time_fmt::day_or_time))))
                        .when(!device.verified, |d| d.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| { this.verification.begin(index); cx.notify(); })))
                })))
            .children(delete_dialog)
            .into_any_element()
    }
}
