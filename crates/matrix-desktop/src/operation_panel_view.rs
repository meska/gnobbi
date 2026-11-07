//! Linux semantic bridge for the inline action bar (not a modal sheet).
use crate::*;
use matrix_desktop::accessibility::operation_panel::{PanelAction, PanelIntent, PanelSnapshot};

pub(crate) struct OperationUi {
    pub preview_focus: FocusHandle,
    preview_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub end_focus: FocusHandle,
    end_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub redact_focus: FocusHandle,
    redact_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub focus: FocusHandle,
    bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub reply_focus: FocusHandle,
    reply_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub edit_focus: FocusHandle,
    edit_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub retry_focus: FocusHandle,
    retry_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
    pub react_focus: FocusHandle,
    react_bounds: Option<(matrix_domain::ScopedTimelineId, u64, accesskit::Rect)>,
}
impl OperationUi {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            preview_focus: cx.focus_handle(),
            preview_bounds: None,
            end_focus: cx.focus_handle(),
            end_bounds: None,
            redact_focus: cx.focus_handle(),
            redact_bounds: None,
            focus: cx.focus_handle(),
            bounds: None,
            reply_focus: cx.focus_handle(),
            reply_bounds: None,
            edit_focus: cx.focus_handle(),
            edit_bounds: None,
            retry_focus: cx.focus_handle(),
            retry_bounds: None,
            react_focus: cx.focus_handle(),
            react_bounds: None,
        }
    }
}
impl Shell {
    /// Own open poll the user may delete (the end event needs the same right).
    pub(crate) fn can_end_poll(&self, target: &matrix_domain::ScopedTimelineId, operations: &matrix_desktop::LiveOperations) -> bool {
        operations.can_end_poll(self.timeline.as_ref().and_then(|t| t.poll_of(target)).as_ref())
    }
    pub(crate) fn semantic_operation_panel(&self, window: &Window) -> Option<PanelSnapshot> {
        if !self.connected
            || self.notifications_open
            || self.verification_open
            || (window.viewport_size().width < px(800.) && !self.compact_chat)
            || !self
                .sidebar
                .selected()
                .is_some_and(|room| room.scope == self.active_scope)
        {
            return None;
        }
        let operations = self.operations.borrow();
        let mut panel = PanelSnapshot::project(
            &operations,
            self.operation_ui.focus.is_focused(window),
            None,
        )?;
        if panel.target.scope != self.active_scope {
            return None;
        }
        panel.bounds = self
            .operation_ui
            .bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.confirming = self.confirm_redaction.as_ref() == Some(&panel.target);
        panel.end_available = self.can_end_poll(&panel.target, &operations);
        panel.ending = panel.confirming && self.confirm_end;
        if panel.ending {
            // The confirm button of a poll closing sits in the redact slot.
            panel.redact_available = panel.end_available;
        }
        panel.end_focused = self.operation_ui.end_focus.is_focused(window);
        panel.end_bounds = self
            .operation_ui
            .end_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.notice = operations.notice(panel.confirming).into();
        panel.redact_focused = self.operation_ui.redact_focus.is_focused(window);
        panel.redact_bounds = self
            .operation_ui
            .redact_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.preview_focused = self.operation_ui.preview_focus.is_focused(window);
        panel.preview_bounds = self
            .operation_ui
            .preview_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.reply_focused = self.operation_ui.reply_focus.is_focused(window);
        panel.edit_focused = self.operation_ui.edit_focus.is_focused(window);
        panel.react_focused = self.operation_ui.react_focus.is_focused(window);
        panel.retry_focused = self.operation_ui.retry_focus.is_focused(window);
        panel.retry_bounds = self
            .operation_ui
            .retry_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.react_bounds = self
            .operation_ui
            .react_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.edit_bounds = self
            .operation_ui
            .edit_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        panel.reply_bounds = self
            .operation_ui
            .reply_bounds
            .as_ref()
            .filter(|(target, generation, _)| {
                *target == panel.target && *generation == panel.tag.generation()
            })
            .map(|(_, _, bounds)| *bounds);
        Some(panel)
    }
    pub(crate) fn apply_semantic_operation_panel(
        &mut self,
        action: PanelAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .semantic_operation_panel(window)
            .is_some_and(|panel| panel.allows(&action))
        {
            return;
        }
        match action.intent {
            PanelIntent::FocusPreview => self.operation_ui.preview_focus.focus(window),
            PanelIntent::Preview => (),
            PanelIntent::FocusRedact | PanelIntent::FocusConfirmRedact => {
                self.operation_ui.redact_focus.focus(window)
            }
            PanelIntent::FocusEnd => self.operation_ui.end_focus.focus(window),
            PanelIntent::BeginRedact | PanelIntent::BeginEnd => {
                self.confirm_end = matches!(action.intent, PanelIntent::BeginEnd);
                // Rotate the opening before exposing confirmation: queued begin
                // actions and confirmations from an earlier attempt are retired.
                self.operations.borrow_mut().select(action.target.clone());
                self.confirm_redaction = Some(action.target);
                self.operation_ui.focus.focus(window); // safe default: Cancel
            }
            PanelIntent::CancelRedact => {
                self.confirm_redaction = None;
                if std::mem::take(&mut self.confirm_end) {
                    self.operation_ui.end_focus.focus(window);
                } else {
                    self.operation_ui.redact_focus.focus(window);
                }
            }
            PanelIntent::ConfirmRedact => {
                let ending = std::mem::take(&mut self.confirm_end);
                #[cfg(feature = "live")]
                self.operations.borrow_mut().submit(
                    if ending {
                        matrix_engine::operations::Operation::EndPoll
                    } else {
                        matrix_engine::operations::Operation::Redact
                    },
                    matrix_engine::sending::fresh_transaction_id(),
                );
                self.confirm_redaction = None;
                if ending {
                    // Back to the poll message, not the composer.
                    if let Some(timeline) = &self.timeline {
                        timeline.focus_semantic_actions(&action.target, window);
                    }
                } else {
                    self.composer
                        .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
                }
            }
            PanelIntent::FocusRetry => self.operation_ui.retry_focus.focus(window),
            PanelIntent::Retry => {
                self.operations.borrow_mut().retry();
                self.composer
                    .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
            }
            PanelIntent::FocusReact => self.operation_ui.react_focus.focus(window),
            PanelIntent::React => {
                #[cfg(feature = "live")]
                self.operations
                    .borrow_mut()
                    .toggle_reaction("+1", matrix_engine::sending::fresh_transaction_id());
                self.composer
                    .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
            }
            PanelIntent::FocusEdit => self.operation_ui.edit_focus.focus(window),
            PanelIntent::FocusReply => self.operation_ui.reply_focus.focus(window),
            PanelIntent::Reply | PanelIntent::Edit => {
                #[cfg(feature = "live")]
                self.composer.update(cx, |editor, cx| {
                    editor.accept_operation(
                        &mut self.operations.borrow_mut(),
                        matches!(action.intent, PanelIntent::Edit),
                        matrix_engine::sending::fresh_transaction_id(),
                        cx,
                    )
                });
                // An accepted write retires its semantic control while pending.
                self.composer
                    .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx));
            }
            PanelIntent::Focus => self.operation_ui.focus.focus(window),
            PanelIntent::Composer => self
                .composer
                .update(cx, |editor, cx| editor.focus_from_accessibility(window, cx)),
            PanelIntent::Message => {
                if let Some(timeline) = &self.timeline {
                    timeline.focus_semantic_actions(&action.target, window);
                }
            }
            PanelIntent::Close => {
                self.operations.borrow_mut().clear_selection();
                self.confirm_redaction = None;
                self.operation_ui.preview_bounds = None;
                self.operation_ui.bounds = None;
                self.operation_ui.reply_bounds = None;
                self.operation_ui.edit_bounds = None;
                self.operation_ui.react_bounds = None;
                self.operation_ui.retry_bounds = None;
                self.operation_ui.redact_bounds = None;
                self.operation_ui.end_bounds = None;
                if let Some(timeline) = &self.timeline {
                    timeline.focus_semantic_actions(&action.target, window);
                }
            }
        }
        cx.notify();
    }
    pub(crate) fn operation_control_observer(
        &self,
        control: &'static str,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let panel = PanelSnapshot::project(&self.operations.borrow(), false, None)?;
        let entity = cx.entity();
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
                    let observed = (panel.target.clone(), panel.tag.generation(), rect);
                    let old = match control {
                        "preview" => &entity.read(cx).operation_ui.preview_bounds,
                        "reply" => &entity.read(cx).operation_ui.reply_bounds,
                        "edit" => &entity.read(cx).operation_ui.edit_bounds,
                        "react" => &entity.read(cx).operation_ui.react_bounds,
                        "retry" => &entity.read(cx).operation_ui.retry_bounds,
                        "redact" | "confirm-redact" => &entity.read(cx).operation_ui.redact_bounds,
                        "end-poll" => &entity.read(cx).operation_ui.end_bounds,
                        _ => &entity.read(cx).operation_ui.bounds,
                    };
                    if old.as_ref() != Some(&observed) {
                        let entity = entity.clone();
                        let tag = panel.tag.clone();
                        window.on_next_frame(move |_, cx| {
                            entity.update(cx, |this, cx| {
                                if !tag.is_cancelled() {
                                    match control {
                                        "preview" => {
                                            this.operation_ui.preview_bounds = Some(observed)
                                        }
                                        "reply" => this.operation_ui.reply_bounds = Some(observed),
                                        "edit" => this.operation_ui.edit_bounds = Some(observed),
                                        "react" => this.operation_ui.react_bounds = Some(observed),
                                        "retry" => this.operation_ui.retry_bounds = Some(observed),
                                        "redact" | "confirm-redact" => {
                                            this.operation_ui.redact_bounds = Some(observed)
                                        }
                                        "end-poll" => this.operation_ui.end_bounds = Some(observed),
                                        _ => this.operation_ui.bounds = Some(observed),
                                    }
                                    cx.notify();
                                }
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
}
