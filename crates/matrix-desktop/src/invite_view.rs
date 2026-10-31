//! Invitations in the room list: «Accetta» / «Rifiuta» on the row, Invio and Canc on the cursor row.
//! The click is the only trigger; the SDK does the join/leave and the next sync updates the list.
use crate::*;
use matrix_desktop::{DesignTokens, InviteState};
use matrix_engine::operations::{OperationCommand, OperationResult};

/// Row subtitle for an invitation.
pub(crate) fn preview(state: InviteState) -> &'static str {
    match state {
        InviteState::Idle => "Invito",
        InviteState::ConfirmReject => "Rifiutare?",
        InviteState::Busy => "Attendi…",
        InviteState::Failed => "Non riuscito",
    }
}

impl Shell {
    /// A click (or key) on «Accetta» / «Rifiuta». Rejecting asks a second click before leaving.
    pub(crate) fn answer_invite(&mut self, scope: RoomScope, accept: bool, cx: &mut Context<Self>) {
        if self.pending_operation_command.is_none()
            && let Some(accept) = self.sidebar.answer_invite(&scope, accept)
        {
            self.pending_operation_command =
                Some(OperationCommand::AnswerInvite { scope, accept });
        }
        cx.notify();
    }

    pub(crate) fn invite_answered(&mut self, result: OperationResult, cx: &mut Context<Self>) {
        let OperationResult::InviteAnswered { scope, accept, result } = result else {
            return;
        };
        self.sidebar.invite_answered(&scope, result.is_ok());
        if result.is_ok() && accept {
            self.pending_open = Some(scope);
            self.open_started_room(cx);
        }
        cx.notify();
    }

    /// The two buttons of an invitation row (nothing while the answer is on its way).
    pub(crate) fn invite_buttons(
        &self,
        scope: &RoomScope,
        state: InviteState,
        t: DesignTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if state == InviteState::Busy {
            return div().into_any_element();
        }
        let button = |id: &'static str, label: &'static str, accept: bool, primary: bool| {
            let scope = scope.clone();
            div()
                .id(gpui::SharedString::from(format!("{id}:{}", scope.room_id.as_str())))
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.))
                .when(primary, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
                .when(!primary, |d| d.border_1().border_color(rgb(t.secondary)).text_color(rgb(t.secondary)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.answer_invite(scope.clone(), accept, cx);
                }))
        };
        div()
            .flex()
            .gap_1()
            .flex_shrink_0()
            .child(button(
                "invite-accept",
                if state == InviteState::Failed { "Riprova" } else { "Accetta" },
                true,
                true,
            ))
            .child(button(
                "invite-reject",
                if state == InviteState::ConfirmReject { "Conferma" } else { "Rifiuta" },
                false,
                false,
            ))
            .into_any_element()
    }
}
