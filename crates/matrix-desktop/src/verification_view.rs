//! Original device verification screen; no IO or trust decisions during rendering.
use crate::*;
use matrix_engine::verification::VerificationAction;

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
pub(crate) struct DeviceScroll {
    handle: gpui::ScrollHandle,
    anchors: [gpui::ScrollAnchor; 6],
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

impl Shell {
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_devices(
        &self,
        window: &Window,
    ) -> matrix_desktop::accessibility::devices::DeviceView {
        use matrix_desktop::accessibility::devices::{DeviceOverview, DeviceView};
        if !self.connected || self.notifications_open || self.room_details_open {
            DeviceView::Unavailable
        } else if self.verification_open {
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
            DeviceAction::Open(_) => {
                self.set_verification_open(true);
                self.device_focus = Default::default();
                self.a11y_device_bounds = [None; 6];
                self.verification.refresh();
            }
            DeviceAction::Close(_) => self.set_verification_open(false),
            DeviceAction::Focus { control, .. } => {
                self.device_focus = control;
                if control != DeviceControl::Close {
                    let anchor = match control {
                        DeviceControl::Device(id) => {
                            self.device_scroll.rows.get(&id).map(|r| r.0.clone())
                        }
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
            DeviceAction::Decide { action, .. } => {
                self.verification.act(action);
                self.device_focus = DeviceControl::Close;
            }
        }
        self.list_focus.focus(window);
        cx.notify();
    }

    fn device_anchor(&self, _index: usize) -> Option<gpui::ScrollAnchor> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            if _index < 6 {
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
                        let previous = if _index < 6 {
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
                                    if _index < 6 {
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
        div().size_full().flex().flex_col().bg(rgb(t.surface)).text_color(rgb(t.text))
            .font_family(matrix_desktop::UI_FONT).text_size(px(14.))
            .track_focus(&self.list_focus)
            .capture_key_down(cx.listener(Self::navigation_key))
            .child(div().h(px(64.)).flex_shrink_0().px_4().flex().items_center().justify_between().border_b_1().border_color(rgb(t.border))
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Verifica dispositivi"))
                .child(div().id("verification-close").relative().children(self.device_observer(0, cx))
                    .when(focused_control == Some(0), |d| d.border_1().border_color(rgb(t.accent)))
                    .cursor_pointer().text_color(rgb(t.accent)).child("Chiudi · Esc")
                    .on_click(cx.listener(|this, _, _, cx| { this.set_verification_open(false); cx.notify(); }))))
            .child(div().id("verification-content").flex_1().min_h_0().overflow_y_scroll()
                .when_some(self.device_scroll_handle(), |d, handle| d.track_scroll(&handle))
                .p_4().flex().flex_col().gap_3()
                .when_some(panel.own_device(), |d, id| d.child(div().w_full().flex_shrink_0().min_h(px(56.)).px_3().py_2().rounded_lg().bg(rgb(t.input)).flex().flex_col().gap_1()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(match panel.own() {
                        Some(own) if own.display_name.is_some() => format!("Questo dispositivo · {}", matrix_desktop::device_title(own)),
                        _ => "Questo dispositivo".to_owned(),
                    }))
                    .child(div().min_w_0().text_size(px(12.)).child(matrix_desktop::row_text::one_line(&match panel.own() {
                        Some(own) => matrix_desktop::device_detail(own, matrix_desktop::time_fmt::day_or_time),
                        None => id.to_owned(),
                    })))))
                .child("Altri dispositivi")
                .child(div().id("verification-refresh").anchor_scroll(self.device_anchor(1)).relative().children(self.device_observer(1, cx))
                    .when(focused_control == Some(1), |d| d.border_1().border_color(rgb(t.accent)))
                    .cursor_pointer().text_color(rgb(t.accent)).child("Aggiorna dispositivi")
                    .on_click(cx.listener(|this, _, _, cx| { this.verification.refresh(); cx.notify(); })))
                .when(panel.devices().is_empty(), |d| d.child("Nessun altro dispositivo disponibile"))
                .children(panel.devices().iter().enumerate().map(|(index, device)| {
                    let row_index = semantic_row_ids.get(index).copied();
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    let row_focused = row_index.is_some_and(|id| self.device_focus == matrix_desktop::accessibility::devices::DeviceControl::Device(accesskit::NodeId(id as u64))) && panel.can_begin();
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    let row_focused = false;
                    div().id(("verification-device", index)).relative()
                        .anchor_scroll(row_index.and_then(|id| self.device_anchor(id)))
                        .children(row_index.and_then(|id| self.device_observer(id, cx)))
                        .when(row_focused, |d| d.border_1().border_color(rgb(t.accent)))
                        // flex_shrink_0 + no overflow clip on the row: the scroll area scrolls, rows keep their content height.
                        .w_full().flex_shrink_0().min_h(px(56.)).px_3().py_2().rounded_lg().bg(rgb(t.input))
                        .flex().flex_col().gap_1()
                        .child(div().flex().justify_between().gap_2()
                            .child(div().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD).child(matrix_desktop::row_text::one_line(matrix_desktop::device_title(device))))
                            .when(!device.verified, |d| d.child(div().text_color(rgb(t.accent)).child("Verifica"))))
                        .child(div().min_w_0().text_size(px(12.)).text_color(rgb(if device.verified { t.secondary } else { t.error }))
                            .child(matrix_desktop::row_text::one_line(&matrix_desktop::device_detail(device, matrix_desktop::time_fmt::day_or_time))))
                        .when(!device.verified, |d| d.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| { this.verification.begin(index); cx.notify(); })))
                }))
                .child(div().border_t_1().border_color(rgb(t.border)).pt_3().font_weight(gpui::FontWeight::SEMIBOLD).child(panel.status_label()))
                .when_some(panel.snapshot(), |d, snapshot| d
                    .child(div().text_size(px(12.)).truncate().child(matrix_desktop::row_text::one_line(&snapshot.user_id)))
                    // Wraps, not truncated: the full ID ellipsized in the list is readable here once the device is selected.
                    .child(div().text_size(px(12.)).child(matrix_desktop::row_text::one_line(&format!("Dispositivo: {}", snapshot.device_id)))))
                .child(div().text_color(rgb(if panel.verified() { t.accent } else { t.error })).child(panel.trust_label()))
                .when_some(panel.comparison(), |d, (a,b,c)| d.child(div().id("verification-comparison").anchor_scroll(self.device_anchor(5)).relative().children(self.device_observer(5, cx))
                    .when(focused_control == Some(5), |d| d.border_1().border_color(rgb(t.accent)))
                    .text_size(px(24.)).font_weight(gpui::FontWeight::SEMIBOLD).child(format!("{a}   {b}   {c}"))))
                .children([(VerificationAction::Confirm, "Coincidono"), (VerificationAction::Mismatch, "Non coincidono"), (VerificationAction::Cancel, "Annulla verifica")].into_iter().enumerate().map(|(index,(action,label))| {
                    let enabled = panel.can_act(action);
                    div().id(label).anchor_scroll(self.device_anchor(index + 2)).relative().children(self.device_observer(index + 2, cx))
                        .when(enabled && focused_control == Some(index + 2), |d| d.border_1().border_color(rgb(t.accent)))
                        .px_3().py_2().rounded_lg().bg(rgb(t.input))
                        .text_color(rgb(if enabled { t.accent } else { t.secondary }))
                        .when(enabled, |d| d.cursor_pointer())
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| { this.verification.act(action); cx.notify(); }))
                })))
            .into_any_element()
    }
}
