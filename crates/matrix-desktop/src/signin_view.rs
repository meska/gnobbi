//! Graphical projection only; authentication remains on RoomListWorker.
use gpui::{ClipboardItem, Context, FocusHandle, KeyDownEvent, Window, div, prelude::*, px, rgb};
use matrix_desktop::{DesignTokens, FormStatus, LIGHT, LoginInput, SetupPhase, SignInForm, visible_fields};
use matrix_engine::signin::{
    RecoveryState, SetupEvent, SetupFailure, SignInError, SignInEvent, SignInMode, SignInUi,
};

#[cfg(test)]
type PaintedSubmit = std::rc::Rc<dyn Fn(&mut SignInView, &mut Context<SignInView>)>;

pub struct SignInView {
    /// Active theme, pushed by the shell on every frame.
    pub theme: DesignTokens,
    form: SignInForm,
    route: SignInUi,
    fields: [gpui::Entity<crate::login_field::LoginField>; 4],
    mode: SignInMode,
    /// Account of a vault sign-in, handed over only once the recovery step is deferred.
    held: Option<matrix_domain::AccountId>,
    fields_source: Option<matrix_desktop::SignInSource>,
    submit_focus: FocusHandle,
    recover_focus: FocusHandle,
    copy_focus: FocusHandle,
    focus_next: Option<usize>,
    #[cfg(test)]
    painted_submit: Option<PaintedSubmit>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    button_bounds: [Option<accesskit::Rect>; 3],
}
impl SignInView {
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "tab" {
            let mut ring = if self.form.recovery().is_some() {
                if self.form.setup_phase() == Some(SetupPhase::KeyShown) {
                    vec![7]
                } else if self.key_route() {
                    vec![3, 5]
                } else if self.form.setup_offered() {
                    vec![5]
                } else {
                    vec![]
                }
            } else {
                visible_fields(self.mode, self.form.vault).to_vec()
            };
            ring.push(4);
            let current = self
                .fields
                .iter()
                .position(|field| field.read(cx).focus.is_focused(window))
                .unwrap_or(if self.recover_focus.is_focused(window) {
                    5
                } else if self.copy_focus.is_focused(window) {
                    7
                } else {
                    4
                });
            let position = ring
                .iter()
                .position(|index| *index == current)
                .unwrap_or(ring.len() - 1);
            let next = if event.keystroke.modifiers.shift {
                (position + ring.len() - 1) % ring.len()
            } else {
                (position + 1) % ring.len()
            };
            if ring[next] == 4 {
                self.submit_focus.focus(window);
            } else if ring[next] == 5 {
                self.recover_focus.focus(window);
            } else if ring[next] == 7 {
                self.copy_focus.focus(window);
            } else {
                self.fields[ring[next]].read(cx).focus.focus(window);
            }
            cx.notify();
            cx.stop_propagation();
        } else if event.keystroke.key == "enter"
            || (event.keystroke.key == "space"
                && (self.submit_focus.is_focused(window)
                    || self.recover_focus.is_focused(window)
                    || self.copy_focus.is_focused(window)))
        {
            // Enter alone never confirms a shown key: only the focused confirm button does.
            if self.copy_focus.is_focused(window) {
                self.copy_key(cx);
            } else if self.form.recovery().is_some() && !self.submit_focus.is_focused(window) {
                self.recover(cx);
            } else {
                self.submit(cx);
            }
            cx.stop_propagation();
        }
    }
    fn key_callback(
        &self,
    ) -> impl Fn(&mut Self, &KeyDownEvent, &mut Window, &mut Context<Self>) + use<> {
        let source = self.form.source();
        move |this, event, window, cx| {
            if source.is_some() && this.form.source() == source {
                this.key(event, window, cx);
            }
        }
    }
    fn submit_callback(&self) -> impl Fn(&mut Self, &mut Context<Self>) + use<> {
        let source = self.form.source();
        move |this, cx| {
            if let Some(source) = source {
                this.submit_source(source, cx);
            }
        }
    }
    fn recover_callback(&self) -> impl Fn(&mut Self, &mut Context<Self>) + use<> {
        let source = self.form.source();
        move |this, cx| {
            if let Some(source) = source {
                this.recover_source(source, cx);
            }
        }
    }
    fn copy_callback(&self) -> impl Fn(&mut Self, &mut Context<Self>) + use<> {
        let source = self.form.source();
        move |this, cx| {
            if source.is_some() && this.form.source() == source {
                this.copy_key(cx);
            }
        }
    }
    /// Explicit user copy of the shown key; nothing else puts it on the clipboard.
    fn copy_key(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.form.recovery_key() {
            cx.write_to_clipboard(ClipboardItem::new_string(key.to_owned()));
        }
    }
    fn recover(&mut self, cx: &mut Context<Self>) {
        (self.recover_callback())(self, cx);
    }
    /// The recovery key can be entered unless the account has no secret storage.
    fn key_route(&self) -> bool {
        self.form.recovery() != Some(RecoveryState::Unsupported)
    }
    fn recover_source(&mut self, source: matrix_desktop::SignInSource, cx: &mut Context<Self>) {
        if self.form.recovery() == Some(RecoveryState::Unsupported) {
            if let Some(command) = self.form.setup_source(source)
                && self.route.commands.try_send(command).is_err()
            {
                self.form
                    .apply_setup(SetupEvent::Failed(SetupFailure::Other));
            }
            self.update_enabled(cx);
            cx.notify();
            return;
        }
        if self.fields[3].read(cx).input.marked_utf16().is_some() {
            return;
        }
        self.form.phrase = self.fields[3].update(cx, |field, cx| {
            cx.notify();
            std::mem::replace(&mut field.input, LoginInput::secret())
        });
        if let Some(command) = self.form.recover_source(source)
            && self.route.commands.try_send(command).is_err()
        {
            self.form.apply(SignInEvent::Recovery(RecoveryState::Failed(
                matrix_engine::signin::RecoveryFailure::Other,
            )));
        }
        self.update_enabled(cx);
        cx.notify();
    }
    fn submit(&mut self, cx: &mut Context<Self>) {
        (self.submit_callback())(self, cx);
    }
    fn submit_source(&mut self, source: matrix_desktop::SignInSource, cx: &mut Context<Self>) {
        if self.form.recovery().is_some() {
            let command = if self.form.setup_phase() == Some(SetupPhase::KeyShown) {
                self.form.confirm_saved_source(source)
            } else {
                self.form.defer_source(source)
            };
            if let Some(command) = command
                && self.route.commands.try_send(command).is_err()
            {
                self.form.apply(SignInEvent::Failed(SignInError::NoSession));
            }
            self.update_enabled(cx);
            cx.notify();
            return;
        }
        if self.form.source() != Some(source)
            || self
                .fields
                .iter()
                .any(|field| field.read(cx).input.marked_utf16().is_some())
        {
            return;
        }
        self.form.homeserver = self.fields[0].read(cx).input.display_text();
        self.form.user = self.fields[1].read(cx).input.display_text();
        let missing_password = self.mode == SignInMode::Password
            && self.fields[2].read(cx).input.display_text().is_empty()
            && !self.fields[3].read(cx).input.display_text().is_empty();
        self.form.password = self.fields[2].update(cx, |field, cx| {
            cx.notify();
            std::mem::replace(&mut field.input, LoginInput::secret())
        });
        self.form.phrase = self.fields[3].update(cx, |field, cx| {
            cx.notify();
            std::mem::replace(&mut field.input, LoginInput::secret())
        });
        if let Some(command) = self.form.submit_source(source)
            && self.route.commands.try_send(command).is_err()
        {
            self.form.apply(SignInEvent::Failed(SignInError::NoSession));
        }
        self.update_enabled(cx);
        if missing_password
            && matches!(self.form.status(), FormStatus::Prompt(prompt) if prompt.error == Some(SignInError::InvalidSecret))
        {
            self.focus_next = Some(2);
        }
        cx.notify();
    }
    fn update_enabled(&mut self, cx: &mut Context<Self>) {
        if self.fields_source != self.form.source() {
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            {
                self.button_bounds = [None; 3];
            }
            let public: [String; 2] =
                std::array::from_fn(|i| self.fields[i].read(cx).input.display_text());
            // Retired GPUI input handlers retain their entity, never the next prompt's editors.
            for (i, field) in self.fields.iter().enumerate() {
                field.update(cx, |field, cx| {
                    field.enabled = false;
                    if i >= 2 {
                        field.input = LoginInput::secret();
                    }
                    cx.notify();
                });
            }
            self.fields = std::array::from_fn(|i| {
                let field = cx.new(|cx| {
                    let mut field = crate::login_field::LoginField::new(i >= 2, cx);
                    if i < 2 {
                        field.input.replace(None, &public[i]);
                    }
                    field
                });
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            });
            self.fields_source = self.form.source();
            self.focus_next = None;
            if let FormStatus::Prompt(prompt) = self.form.status() {
                self.focus_next = Some(if prompt.mode == SignInMode::UnlockVault {
                    4
                } else if prompt.mode == SignInMode::Unlock {
                    3
                } else {
                    match prompt.error {
                        None | Some(SignInError::InvalidHomeserver) => 0,
                        Some(SignInError::InvalidUserId) => 1,
                        Some(SignInError::InvalidSecret) => 3,
                        _ => 2,
                    }
                });
            } else if self.form.recovery().is_some() {
                self.focus_next = Some(
                    if matches!(
                        self.form.recovery(),
                        Some(RecoveryState::Offered | RecoveryState::Failed(_))
                    ) {
                        3
                    } else if self.form.setup_phase() == Some(SetupPhase::KeyShown) {
                        7
                    } else if self.form.setup_offered() {
                        5
                    } else {
                        4
                    },
                );
            }
        }
        let enabled = matches!(self.form.status(), FormStatus::Prompt(_));
        let visible = visible_fields(self.mode, self.form.vault);
        let key_open = self.form.source().is_some()
            && matches!(
                self.form.recovery(),
                Some(RecoveryState::Offered | RecoveryState::Failed(_))
            );
        for (index, field) in self.fields.iter().enumerate() {
            field.update(cx, |field, cx| {
                field.enabled = if self.form.recovery().is_some() {
                    key_open && index == 3
                } else {
                    enabled && visible.contains(&index)
                };
                cx.notify();
            });
        }
    }
    /// New profiles use the OS vault: no local phrase.
    pub fn with_vault(mut self, vault: bool) -> Self {
        self.form.vault = vault;
        self
    }
    pub fn new(route: SignInUi, cx: &mut Context<Self>) -> Self {
        Self {
            theme: LIGHT,
            form: SignInForm::default(),
            route,
            mode: SignInMode::Password,
            held: None,
            fields_source: None,
            submit_focus: cx.focus_handle(),
            recover_focus: cx.focus_handle(),
            copy_focus: cx.focus_handle(),
            focus_next: None,
            #[cfg(test)]
            painted_submit: None,
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            button_bounds: [None; 3],
            fields: std::array::from_fn(|index| {
                let field = cx.new(|cx| crate::login_field::LoginField::new(index >= 2, cx));
                cx.observe(&field, |_, _, cx| cx.notify()).detach();
                field
            }),
        }
    }
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn semantic_snapshot(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> matrix_desktop::accessibility::signin::Snapshot {
        use matrix_desktop::accessibility::signin::{Control, Snapshot};
        let mut snapshot = Snapshot::new(&self.form, self.mode);
        snapshot.public = std::array::from_fn(|i| self.fields[i].read(cx).input.display_text());
        for (i, field) in self.fields.iter().enumerate() {
            snapshot.bounds[i] = field.read(cx).semantic_bounds;
            if field.read(cx).focus.is_focused(window) {
                snapshot.focused = Some(Control::ALL[i]);
            }
        }
        let button = if snapshot.recovery.is_some() {
            Control::Defer
        } else {
            Control::Submit
        };
        snapshot.bounds[button as usize] = self.button_bounds[0];
        snapshot.bounds[Control::Recover as usize] = self.button_bounds[1];
        snapshot.bounds[Control::Copy as usize] = self.button_bounds[2];
        if self.submit_focus.is_focused(window) {
            snapshot.focused = Some(button);
        } else if self.recover_focus.is_focused(window) {
            snapshot.focused = Some(Control::Recover);
        } else if self.copy_focus.is_focused(window) {
            snapshot.focused = Some(Control::Copy);
        }
        snapshot
    }
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn apply_semantic(
        &mut self,
        intent: matrix_desktop::accessibility::signin::Intent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use matrix_desktop::accessibility::signin::Control;
        let allowed = if self.form.recovery().is_some() {
            intent.control == Control::Defer
                || (self.key_route()
                    && matches!(intent.control, Control::Recover | Control::Phrase))
                || (self.form.setup_offered() && intent.control == Control::Recover)
                || (self.form.setup_phase() == Some(SetupPhase::KeyShown)
                    && intent.control == Control::Copy)
        } else {
            intent.control == Control::Submit
                || visible_fields(self.mode, self.form.vault).contains(&(intent.control as usize))
        };
        if self.form.source() != Some(intent.source) || !allowed {
            return false;
        }
        if intent.control == Control::Copy {
            if intent.focus_only {
                self.copy_focus.focus(window);
            } else {
                self.copy_key(cx);
            }
        } else if intent.control == Control::Recover {
            if intent.focus_only {
                self.recover_focus.focus(window);
            } else {
                self.recover_source(intent.source, cx);
            }
        } else if matches!(intent.control, Control::Submit | Control::Defer) {
            if intent.focus_only {
                self.submit_focus.focus(window);
            } else {
                self.submit_source(intent.source, cx);
            }
        } else {
            self.fields[intent.control as usize]
                .read(cx)
                .focus
                .focus(window);
        }
        cx.notify();
        true
    }
    fn bounds_observer(&self, _slot: usize, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let entity = _cx.entity();
            let source = self.form.source();
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
                        if entity.read(cx).button_bounds[_slot] != Some(rect) {
                            let entity = entity.clone();
                            cx.defer(move |cx| {
                                entity.update(cx, |this, cx| {
                                    if this.form.source() == source {
                                        this.button_bounds[_slot] = Some(rect);
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
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        {
            None
        }
    }
    pub fn poll(&mut self, cx: &mut Context<Self>) -> Option<matrix_domain::AccountId> {
        if let Ok(event) = self.route.setup.try_recv() {
            self.form.apply_setup(event);
            self.update_enabled(cx);
            cx.notify();
            return None;
        }
        if let Ok(event) = self.route.events.try_recv() {
            for field in &self.fields[2..] {
                field.update(cx, |field, cx| {
                    field.input = LoginInput::secret();
                    cx.notify();
                });
            }
            if let SignInEvent::Prompt(prompt) = event {
                self.mode = prompt.mode;
            }
            // Authenticated before any prompt: the backend unlocked the vault by itself.
            let unprompted = self.form.status() == FormStatus::Opening;
            self.form.apply(event);
            self.update_enabled(cx);
            cx.notify();
            match event {
                // A vault sign-in still owes the recovery step: keep the view alive until it is deferred.
                SignInEvent::Authenticated(account)
                    if self.form.vault && self.mode == SignInMode::Password && !unprompted =>
                {
                    self.held = Some(account);
                }
                SignInEvent::Authenticated(account) => return Some(account),
                SignInEvent::Recovery(RecoveryState::Deferred | RecoveryState::Recovered) => {
                    return self.held.take();
                }
                _ => (),
            }
        }
        None
    }
}
impl Render for SignInView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        for field in &self.fields {
            field.update(cx, |f, _| f.theme = t);
        }
        if let Some(index) = self.focus_next.take() {
            match index {
                4 => self.submit_focus.focus(window),
                5 => self.recover_focus.focus(window),
                7 => self.copy_focus.focus(window),
                _ => self.fields[index].read(cx).focus.focus(window),
            }
        }
        let recovery = self.form.recovery().is_some();
        let unlock = self.mode != SignInMode::Password;
        let error = match self.form.status() {
            FormStatus::Prompt(prompt) => prompt.error,
            FormStatus::Failed(error) => Some(error),
            _ => None,
        };
        let section = if recovery {
            "Cronologia cifrata"
        } else if unlock {
            "Sblocca"
        } else {
            "Accedi"
        };
        window.set_window_title(&matrix_desktop::window::title_for(Some((
            section,
            error.is_some()
                || matches!(self.form.recovery(), Some(RecoveryState::Failed(_)))
                || matches!(self.form.setup_phase(), Some(SetupPhase::Failed(_))),
        ))));
        let enabled = matches!(self.form.status(), FormStatus::Prompt(_))
            || (recovery && self.form.source().is_some());
        let key = self.key_callback();
        let submit = std::rc::Rc::new(self.submit_callback());
        #[cfg(test)]
        {
            self.painted_submit = Some(submit.clone());
        }
        let mut card = div()
            .w_full()
            .max_w(px(420.))
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_size(px(24.)).child(if recovery {
                "Cronologia cifrata"
            } else if unlock {
                "Sblocca Gnobbi"
            } else {
                "Accedi a Gnobbi"
            }));
        for (index, label) in [
            "Homeserver HTTPS",
            "ID Matrix completo",
            "Password Matrix",
            "Frase locale · almeno 16 caratteri",
        ]
        .into_iter()
        .enumerate()
        .filter(|(index, _)| {
            !recovery && visible_fields(self.mode, self.form.vault).contains(index)
        }) {
            card = card
                .child(div().text_size(px(12.)).child(label))
                .child(self.fields[index].clone());
        }
        if recovery {
            let state = self.form.recovery();
            let key_open = self.form.source().is_some() && self.key_route();
            let phase = self.form.setup_phase();
            if let Some(key) = self.form.recovery_key() {
                let copy = std::rc::Rc::new(self.copy_callback());
                card = card
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(matrix_desktop::SETUP_KEY_NOTE),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(matrix_desktop::SETUP_KEY_LABEL),
                    )
                    .child(
                        div()
                            .id("signin-setup-key")
                            .p_3()
                            .border_2()
                            .border_color(rgb(t.accent))
                            .font_family(matrix_desktop::MONO_FONT)
                            .text_size(px(15.))
                            .child(key.to_owned()),
                    )
                    .child(
                        div()
                            .id("signin-copy")
                            .relative()
                            .children(self.bounds_observer(2, cx))
                            .track_focus(&self.copy_focus)
                            .h(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_2()
                            .border_color(rgb(if self.copy_focus.is_focused(window) {
                                t.text
                            } else {
                                t.accent
                            }))
                            .text_color(rgb(t.accent))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| copy(this, cx)))
                            .child(matrix_desktop::COPY_LABEL),
                    );
            } else {
                card = card
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(matrix_desktop::RECOVERY_WARNING),
                    )
                    .child(div().text_size(px(12.)).child(if self.key_route() {
                        matrix_desktop::RECOVERY_NOTE
                    } else {
                        matrix_desktop::SETUP_NOTE
                    }));
            }
            if self.key_route() {
                let recover = std::rc::Rc::new(self.recover_callback());
                card = card
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(matrix_desktop::RECOVERY_KEY_LABEL),
                    )
                    .child(self.fields[3].clone())
                    .child(
                        div()
                            .id("signin-recover")
                            .relative()
                            .children(self.bounds_observer(1, cx))
                            .track_focus(&self.recover_focus)
                            .h(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_2()
                            .border_color(rgb(if self.recover_focus.is_focused(window) {
                                t.text
                            } else {
                                t.accent
                            }))
                            .bg(rgb(if key_open {
                                t.accent
                            } else {
                                t.disabled
                            }))
                            .text_color(rgb(if key_open {
                                t.on_accent
                            } else {
                                t.secondary
                            }))
                            .when(key_open, |d| {
                                d.cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| recover(this, cx)))
                            })
                            .child(match state {
                                Some(RecoveryState::Pending) => "Verifica in corso…",
                                Some(RecoveryState::Failed(_)) => matrix_desktop::RETRY_LABEL,
                                _ => matrix_desktop::RECOVER_LABEL,
                            }),
                    );
            } else if self.form.setup_offered() || phase == Some(SetupPhase::Pending) {
                // Outlined, not filled: creating a new recovery is not the restore action.
                let offered = self.form.setup_offered() && self.form.source().is_some();
                let recover = std::rc::Rc::new(self.recover_callback());
                card = card.child(
                    div()
                        .id("signin-recover")
                        .relative()
                        .children(self.bounds_observer(1, cx))
                        .track_focus(&self.recover_focus)
                        .h(px(36.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_2()
                        .border_color(rgb(if self.recover_focus.is_focused(window) {
                            t.text
                        } else if offered {
                            t.accent
                        } else {
                            t.disabled
                        }))
                        .text_color(rgb(if offered {
                            t.accent
                        } else {
                            t.secondary
                        }))
                        .when(offered, |d| {
                            d.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| recover(this, cx)))
                        })
                        .child(match phase {
                            Some(SetupPhase::Pending) => "Creazione in corso…",
                            Some(SetupPhase::Failed(_)) => matrix_desktop::SETUP_RETRY_LABEL,
                            _ => matrix_desktop::SETUP_LABEL,
                        }),
                );
            }
        }
        card = card
            .child(
                div()
                    .id("signin-submit")
                    .relative()
                    .children(self.bounds_observer(0, cx))
                    .track_focus(&self.submit_focus)
                    .h(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_2()
                    .border_color(rgb(if self.submit_focus.is_focused(window) {
                        t.text
                    } else {
                        t.accent
                    }))
                    .bg(rgb(if enabled {
                        t.accent
                    } else {
                        t.disabled
                    }))
                    .text_color(rgb(if enabled {
                        t.on_accent
                    } else {
                        t.secondary
                    }))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                submit(this, cx);
                            }))
                    })
                    .child(if enabled {
                        if recovery {
                            if self.form.setup_phase() == Some(SetupPhase::KeyShown) {
                                matrix_desktop::SAVED_LABEL
                            } else {
                                matrix_desktop::DEFER_LABEL
                            }
                        } else if self.mode == SignInMode::UnlockVault {
                            "Riprova"
                        } else if unlock {
                            "Sblocca"
                        } else {
                            "Accedi"
                        }
                    } else {
                        "Attendere…"
                    }),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(
                        if matches!(
                            (self.form.recovery(), self.form.setup_phase()),
                            (Some(RecoveryState::Pending), _)
                                | (
                                    Some(RecoveryState::Unsupported),
                                    None | Some(SetupPhase::Pending)
                                )
                        ) {
                            t.secondary
                        } else {
                            t.error
                        },
                    ))
                    .child(match self.form.recovery() {
                        Some(RecoveryState::Pending) => matrix_desktop::RECOVERY_PENDING,
                        _ => self.form.error_message().unwrap_or(""),
                    }),
            );
        div()
            .id("signin")
            .on_key_down(cx.listener(move |this, event, window, cx| key(this, event, window, cx)))
            .size_full()
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .font_family(matrix_desktop::UI_FONT)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(56.))
            .px_6()
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires private display"]
    fn local_validation_retry_recovers_focus_on_the_new_editor() {
        local_validation_retry_focus(SignInMode::Password);
    }

    #[test]
    #[ignore = "requires private display"]
    fn unlock_local_validation_retry_recovers_focus_on_the_new_editor() {
        local_validation_retry_focus(SignInMode::Unlock);
    }

    fn local_validation_retry_focus(mode: SignInMode) {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            events.try_send(SignInEvent::Prompt(matrix_engine::signin::SignInPrompt { mode, error: None })).unwrap();
            let view = cx.open_window(gpui::WindowOptions::default(), |_, cx| cx.new(|cx| {
                let mut view = SignInView::new(SignInUi { events: event_rx, setup: matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap()).1, commands }, cx);
                view.poll(cx);
                view
            })).unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(250)).await;
                view.update(cx, |v, window, cx| {
                    assert!(v.fields[if mode == SignInMode::Unlock { 3 } else { 0 }].read(cx).focus.is_focused(window));
                    let old = v.painted_submit.clone().expect("initial painted submit");
                    old(v, cx);
                    assert!(requests.try_recv().is_err());
                    assert_eq!(v.focus_next, Some(3), "local InvalidSecret must recover focus after replacing editors");
                    let _ = v.render(window, cx);
                    assert!(v.fields[3].read(cx).focus.is_focused(window));
                    old(v, cx);
                    assert!(requests.try_recv().is_err());
                    if mode == SignInMode::Password {
                    v.fields[3].update(cx, |f,_| { f.input.replace(None, "valid local fixture phrase"); });
                    let current = v.submit_callback();
                    current(v,cx);
                    assert_eq!(v.focus_next,Some(2), "missing password with a valid phrase must focus password, not the cleared phrase");
                    let _ = v.render(window,cx);
                    assert!(v.fields[2].read(cx).focus.is_focused(window));
                    assert!(requests.try_recv().is_err());
                    }
                    if mode == SignInMode::Unlock {
                        for error in [SignInError::InvalidSecret, SignInError::Authentication(matrix_engine::auth::AuthError::Storage)] {
                            let retired = v.fields[3].clone();
                            let stale = v.submit_callback();
                            events.try_send(SignInEvent::Prompt(matrix_engine::signin::SignInPrompt { mode, error: Some(error) })).unwrap();
                            v.poll(cx);
                            assert!(!retired.read(cx).enabled);
                            let _ = v.render(window, cx);
                            assert!(v.fields[3].read(cx).focus.is_focused(window));
                            v.fields[3].update(cx, |f, _| { f.input.replace(None, "protected backend retry phrase"); });
                            stale(v, cx);
                            assert!(requests.try_recv().is_err());
                            assert!(!v.fields[3].read(cx).input.display_text().is_empty());
                        }
                    }
                    eprintln!("initial/local/backend error focus, replacement editors and stale submit checked ({mode:?})");
                }).unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_password_projection_dispatches_only_current_prompt() {
        painted_projection_dispatches_only_current_prompt(SignInMode::Password);
    }

    #[test]
    #[ignore = "requires private display"]
    fn painted_unlock_projection_dispatches_only_current_prompt() {
        painted_projection_dispatches_only_current_prompt(SignInMode::Unlock);
    }

    fn painted_projection_dispatches_only_current_prompt(mode: SignInMode) {
        let start = if mode == SignInMode::Unlock { 3 } else { 0 };
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let prompt = SignInEvent::Prompt(matrix_engine::signin::SignInPrompt { mode, error: None });
            assert!(events.try_send(prompt).is_ok());
            let view = cx.open_window(gpui::WindowOptions::default(), |_, cx| cx.new(|cx| {
                let mut view = SignInView::new(SignInUi { events: event_rx, setup: matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap()).1, commands }, cx);
                view.poll(cx);
                view
            })).unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(250)).await;
                view.update(cx, |v, window, cx| {
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    let intent = {
                        use matrix_desktop::accessibility::signin::{Control, Intent};
                        let snapshot = v.semantic_snapshot(window, cx);
                        assert!(snapshot.bounds[..start].iter().all(Option::is_none) && snapshot.bounds[start..5].iter().all(Option::is_some), "all login bounds must come from actual paint: {:?}, painted={}", snapshot.bounds, v.painted_submit.is_some());
                        assert_eq!(snapshot.focused, Some(Control::ALL[start]));
                        let intent = Intent { source: snapshot.source.unwrap(), control: Control::Phrase, focus_only: true };
                        assert!(v.apply_semantic(intent, window, cx));
                        intent
                    };
                    for f in &v.fields[start..] {
                        f.update(cx, |f, cx| {
                            let pointer = f.painted_pointer.expect("painted pointer");
                            let key = f.painted_key.expect("painted key");
                            f.input.replace(None, "fixture native field");
                            pointer(f, window, cx);
                            assert!(f.focus.is_focused(window));
                            key(f, &KeyDownEvent { keystroke: gpui::Keystroke::parse("home").unwrap(), is_held: false }, window, cx);
                            assert_eq!(f.input.selection_utf16(), 0..0);
                            pointer(f, window, cx);
                            assert!(f.input.selection_utf16().end > 0);
                        });
                    }
                    v.fields[3].read(cx).focus.focus(window);
                    assert!(v.fields[3].read(cx).focus.is_focused(window));
                    let old = v.painted_submit.clone().expect("actual painted submit closure");
                    let old_key = v.key_callback();
                    let fields = v.fields[start..].to_vec();
                    let callbacks: Vec<_> = fields.iter().map(|f| {
                        let f = f.read(cx);
                        (f.painted_pointer.expect("painted pointer"), f.painted_key.expect("painted key"))
                    }).collect();
                    assert!(events.try_send(prompt).is_ok()); v.poll(cx);
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    {
                        use matrix_desktop::accessibility::signin::Intent;
                        assert!(!v.apply_semantic(intent, window, cx));
                        assert!(v.button_bounds.iter().all(Option::is_none), "a replacement prompt must not inherit previous painted submit bounds");
                        assert!(v.apply_semantic(Intent { source: v.form.source().unwrap(), ..intent }, window, cx));
                    }
                    v.fields[3].read(cx).focus.focus(window);
                    for (i, f) in fields.iter().enumerate() {
                        use gpui::EntityInputHandler;
                        f.update(cx, |f,cx| {
                            let before = f.input.display_text();
                            let (pointer, key) = callbacks[i];
                            pointer(f, window, cx);
                            assert!(!f.focus.is_focused(window), "retired pointer must not steal focus");
                            key(f, &KeyDownEvent { keystroke: gpui::Keystroke::parse("backspace").unwrap(), is_held: false }, window, cx);
                            f.replace_text_in_range(None, "retired input sentinel", window, cx);
                            f.replace_and_mark_text_in_range(None, "retired preedit", None, window, cx);
                            assert_eq!(f.input.display_text(), before);
                            assert!(f.input.marked_utf16().is_none());
                        });
                        assert!(!v.fields[start+i].read(cx).input.display_text().contains("retired"));
                    }
                    for (i,text) in [(2,"fixture password"),(3,"fixture local phrase long enough")] {
                        v.fields[i].update(cx, |f,_| { f.input.replace(None,text); });
                    }
                    old_key(v, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                    old(v,cx);
                    assert!(requests.try_recv().is_err());
                    assert!(!v.fields[2].read(cx).input.display_text().is_empty());
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    {
                        use matrix_desktop::accessibility::signin::{Control, Intent};
                        let current = Intent { source: v.form.source().unwrap(), control: Control::Submit, focus_only: false };
                        assert!(v.apply_semantic(current, window, cx));
                        assert!(!v.apply_semantic(current, window, cx));
                    }
                    #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
                    {
                        let current = v.submit_callback();
                        current(v, cx);
                        current(v, cx);
                    }
                    assert!(requests.try_recv().is_ok()); assert!(requests.try_recv().is_err());
                    eprintln!("mode-specific painted bounds; protected roles; current Focus; retired IME/submit and duplicate semantic dispatch rejected");
                    events.try_send(prompt).unwrap();
                    v.poll(cx);
                    crate::navigation_resize_test::resize("360", "600");
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(350)).await;
                view.update(cx, |v, window, cx| {
                    assert_eq!(window.viewport_size().width, px(360.));
                    assert!(v.fields[..start].iter().all(|f| f.read(cx).painted_pointer.is_none()) && v.fields[start..].iter().all(|f| f.read(cx).painted_pointer.is_some()));
                    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                    {
                        let snapshot = v.semantic_snapshot(window, cx);
                        assert!(snapshot.bounds[..start].iter().all(Option::is_none) && snapshot.bounds[start..5].iter().all(Option::is_some), "replacement controls must reacquire paint bounds after native Configure");
                        assert!(snapshot.bounds.iter().flatten().all(|r| r.x0 >= 0. && r.x1 <= 360.));
                    }
                    let source = v.form.source();
                    let fresh = v.painted_submit.clone().unwrap();
                    fresh(v, cx);
                    assert_ne!(v.form.source(), source, "reacquired painted callback must target the replacement prompt");
                    assert!(requests.try_recv().is_err());
                    eprintln!("replacement editor paint/bounds and submit reacquired after native 360px Configure");
                }).unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }

    #[test]
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    #[ignore = "requires private display"]
    fn semantic_lifecycle_rejects_translated_actions_across_every_prompt_boundary() {
        semantic_prompt_lifecycle(SignInMode::Password);
    }

    #[test]
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    #[ignore = "requires private display"]
    fn unlock_semantic_lifecycle_rejects_translated_actions_across_every_prompt_boundary() {
        semantic_prompt_lifecycle(SignInMode::Unlock);
    }

    /// Keeps the recovery step on screen for an external capture (the gui-test script).
    #[test]
    #[ignore = "requires private display"]
    fn vault_recovery_step_stays_visible_for_capture() {
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, _requests) = matrix_engine::bounded(capacity);
            let (mut setup_tx, setup_rx) = matrix_engine::bounded(capacity);
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        SignInView::new(
                            SignInUi {
                                events: event_rx,
                                setup: setup_rx,
                                commands,
                            },
                            cx,
                        )
                        .with_vault(true)
                    })
                })
                .unwrap();
            for event in [
                SignInEvent::Prompt(matrix_engine::signin::SignInPrompt {
                    mode: SignInMode::Password,
                    error: None,
                }),
                SignInEvent::Authenticated(matrix_domain::AccountId::new(1)),
                SignInEvent::Recovery(RecoveryState::Offered),
                SignInEvent::Recovery(match std::env::var("DIME_RECOVERY_CAPTURE").as_deref() {
                    Ok("pending") => RecoveryState::Pending,
                    Ok("invalid") => {
                        RecoveryState::Failed(matrix_engine::signin::RecoveryFailure::InvalidKey)
                    }
                    Ok("unsupported" | "setup" | "setup-key" | "setup-failed") => {
                        RecoveryState::Unsupported
                    }
                    _ => RecoveryState::Offered,
                }),
            ] {
                events.try_send(event).unwrap();
                view.update(cx, |v, _, cx| v.poll(cx)).unwrap();
            }
            let capture = std::env::var("DIME_RECOVERY_CAPTURE").unwrap_or_default();
            let setup = match capture.as_str() {
                "setup-key" => Some(SetupEvent::Ready(
                    matrix_engine::recovery::RecoverySecret::fixture(
                        "EsTc 7Qx9 mK2p LdA4 hW8s Yb3N cR6v ZfJ5",
                    ),
                )),
                "setup-failed" => Some(SetupEvent::Failed(SetupFailure::Network)),
                _ => None,
            };
            if let Some(event) = setup {
                assert!(setup_tx.try_send(event).is_ok());
                view.update(cx, |v, _, cx| v.poll(cx)).unwrap();
            }
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_secs(6)).await;
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn recovery_step_sends_the_key_reports_failures_retries_and_continues_on_success() {
        use matrix_engine::signin::{RecoveryFailure, SignInCommand};
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        SignInView::new(
                            SignInUi {
                                events: event_rx,
                                setup: matrix_engine::bounded(
                                    std::num::NonZeroUsize::new(1).unwrap(),
                                )
                                .1,
                                commands,
                            },
                            cx,
                        )
                        .with_vault(true)
                    })
                })
                .unwrap();
            view.update(cx, |v, window, cx| {
                let account = matrix_domain::AccountId::new(1);
                for event in [
                    SignInEvent::Prompt(matrix_engine::signin::SignInPrompt {
                        mode: SignInMode::Password,
                        error: None,
                    }),
                    SignInEvent::Authenticated(account),
                    SignInEvent::Recovery(RecoveryState::Offered),
                ] {
                    events.try_send(event).unwrap();
                    assert_eq!(v.poll(cx), None);
                    let _ = v.render(window, cx);
                }
                assert!(
                    v.fields[3].read(cx).enabled && v.fields[3].read(cx).focus.is_focused(window)
                );
                let type_key = |v: &mut SignInView, cx: &mut Context<SignInView>| {
                    v.fields[3].update(cx, |f, _| f.input.replace(None, "fixture recovery key"));
                };
                type_key(v, cx);
                v.recover(cx);
                assert!(matches!(
                    requests.try_recv(),
                    Ok(SignInCommand::EnterRecovery(_))
                ));
                v.recover(cx);
                assert!(
                    requests.try_recv().is_err(),
                    "a second activation must not resend the key"
                );
                events
                    .try_send(SignInEvent::Recovery(RecoveryState::Pending))
                    .unwrap();
                assert_eq!(v.poll(cx), None);
                let _ = v.render(window, cx);
                assert!(
                    !v.fields[3].read(cx).enabled,
                    "no editing while the key is checked"
                );
                events
                    .try_send(SignInEvent::Recovery(RecoveryState::Failed(
                        RecoveryFailure::InvalidKey,
                    )))
                    .unwrap();
                assert_eq!(v.poll(cx), None);
                let _ = v.render(window, cx);
                assert!(v.form.error_message().unwrap().contains("non valida"));
                assert!(
                    v.fields[3].read(cx).enabled && v.fields[3].read(cx).focus.is_focused(window)
                );
                assert!(
                    v.fields[3].read(cx).input.display_text().is_empty(),
                    "the failed key is gone"
                );
                type_key(v, cx);
                v.recover(cx);
                assert!(matches!(
                    requests.try_recv(),
                    Ok(SignInCommand::EnterRecovery(_))
                ));
                events
                    .try_send(SignInEvent::Recovery(RecoveryState::Recovered))
                    .unwrap();
                assert_eq!(
                    v.poll(cx),
                    Some(account),
                    "success continues to the room list"
                );
            })
            .unwrap();
            cx.spawn(async move |cx| {
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn recovery_setup_offers_creation_shows_the_key_once_and_needs_the_confirmation() {
        use matrix_engine::signin::SignInCommand;
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let (mut setup, setup_rx) = matrix_engine::bounded(capacity);
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        SignInView::new(
                            SignInUi {
                                events: event_rx,
                                setup: setup_rx,
                                commands,
                            },
                            cx,
                        )
                        .with_vault(true)
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                // Wayland reads the clipboard only through a compositor offer: seed one and let it arrive.
                executor.timer(std::time::Duration::from_millis(300)).await;
                cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("seed".into())))
                    .unwrap();
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, |v, window, cx| {
                    let account = matrix_domain::AccountId::new(1);
                    for event in [
                        SignInEvent::Prompt(matrix_engine::signin::SignInPrompt {
                            mode: SignInMode::Password,
                            error: None,
                        }),
                        SignInEvent::Authenticated(account),
                        SignInEvent::Recovery(RecoveryState::Offered),
                    ] {
                        events.try_send(event).unwrap();
                        assert_eq!(v.poll(cx), None);
                        let _ = v.render(window, cx);
                    }
                    // The restore route offers no creation.
                    v.recover(cx);
                    assert!(
                        requests.try_recv().is_err(),
                        "empty key: nothing is sent and nothing is created"
                    );
                    events
                        .try_send(SignInEvent::Recovery(RecoveryState::Unsupported))
                        .unwrap();
                    assert_eq!(v.poll(cx), None);
                    let _ = v.render(window, cx);
                    assert!(
                        v.recover_focus.is_focused(window),
                        "the setup offer takes focus"
                    );
                    v.recover(cx);
                    assert!(matches!(
                        requests.try_recv(),
                        Ok(SignInCommand::SetupRecovery)
                    ));
                    v.recover(cx);
                    assert!(
                        requests.try_recv().is_err(),
                        "a second activation must not start a second setup"
                    );
                    assert!(setup.try_send(SetupEvent::Pending).is_ok());
                    assert_eq!(v.poll(cx), None);
                    let _ = v.render(window, cx);
                    assert!(
                        setup
                            .try_send(SetupEvent::Failed(SetupFailure::Network))
                            .is_ok()
                    );
                    assert_eq!(v.poll(cx), None);
                    let _ = v.render(window, cx);
                    assert!(
                        v.form
                            .error_message()
                            .unwrap()
                            .contains("non raggiungibile")
                    );
                    assert!(
                        v.recover_focus.is_focused(window),
                        "retry keeps focus on the offer"
                    );
                    v.recover(cx);
                    assert!(
                        matches!(requests.try_recv(), Ok(SignInCommand::SetupRecovery)),
                        "retry after a failure"
                    );
                    assert!(
                        setup
                            .try_send(SetupEvent::Ready(
                                matrix_engine::recovery::RecoverySecret::fixture(
                                    "fixture key 1234"
                                )
                            ))
                            .is_ok()
                    );
                    assert_eq!(v.poll(cx), None);
                    let _ = v.render(window, cx);
                    assert_eq!(v.form.recovery_key(), Some("fixture key 1234"));
                    assert!(
                        v.copy_focus.is_focused(window),
                        "focus lands on copy, never on the confirmation"
                    );
                    v.copy_key(cx);
                    assert_eq!(
                        cx.read_from_clipboard()
                            .and_then(|item| item.text())
                            .as_deref(),
                        Some("fixture key 1234")
                    );
                    let stale = v.submit_callback();
                    v.copy_focus.focus(window);
                    v.key(
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse("enter").unwrap(),
                            is_held: false,
                        },
                        window,
                        cx,
                    );
                    assert!(
                        requests.try_recv().is_err(),
                        "Enter on copy must not confirm the key as saved"
                    );
                    assert_eq!(v.form.recovery_key(), Some("fixture key 1234"));
                    v.recover(cx);
                    assert!(
                        requests.try_recv().is_err(),
                        "no second setup while the key is shown"
                    );
                    stale(v, cx);
                    v.submit(cx);
                    assert!(matches!(
                        requests.try_recv(),
                        Ok(SignInCommand::DeferRecovery)
                    ));
                    assert_eq!(
                        v.form.recovery_key(),
                        None,
                        "the key is gone after the confirmation"
                    );
                    v.submit(cx);
                    assert!(requests.try_recv().is_err());
                    events
                        .try_send(SignInEvent::Recovery(RecoveryState::Deferred))
                        .unwrap();
                    assert_eq!(v.poll(cx), Some(account));
                })
                .unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn automatic_vault_unlock_hands_over_the_account_without_any_prompt() {
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, _requests) = matrix_engine::bounded(capacity);
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        SignInView::new(
                            SignInUi {
                                events: event_rx,
                                setup: matrix_engine::bounded(
                                    std::num::NonZeroUsize::new(1).unwrap(),
                                )
                                .1,
                                commands,
                            },
                            cx,
                        )
                        .with_vault(true)
                    })
                })
                .unwrap();
            view.update(cx, |v, _, cx| {
                let account = matrix_domain::AccountId::new(1);
                events
                    .try_send(SignInEvent::Authenticated(account))
                    .unwrap();
                assert_eq!(
                    v.poll(cx),
                    Some(account),
                    "no unlock prompt, no recovery wait"
                );
            })
            .unwrap();
            cx.spawn(async move |cx| {
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn vault_recovery_step_holds_the_account_until_the_defer_button_is_used() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        SignInView::new(
                            SignInUi {
                                events: event_rx,
                                setup: matrix_engine::bounded(
                                    std::num::NonZeroUsize::new(1).unwrap(),
                                )
                                .1,
                                commands,
                            },
                            cx,
                        )
                        .with_vault(true)
                    })
                })
                .unwrap();
            view.update(cx, |v, window, cx| {
                let account = matrix_domain::AccountId::new(1);
                events
                    .try_send(SignInEvent::Prompt(matrix_engine::signin::SignInPrompt {
                        mode: SignInMode::Password,
                        error: None,
                    }))
                    .unwrap();
                assert_eq!(v.poll(cx), None);
                let _ = v.render(window, cx);
                // The phrase editor is not part of the vault form.
                assert!(!v.fields[3].read(cx).enabled);
                events
                    .try_send(SignInEvent::Authenticated(account))
                    .unwrap();
                assert_eq!(v.poll(cx), None, "the account waits for the recovery step");
                events
                    .try_send(SignInEvent::Recovery(RecoveryState::Offered))
                    .unwrap();
                assert_eq!(v.poll(cx), None);
                let _ = v.render(window, cx);
                assert!(
                    v.fields[3].read(cx).focus.is_focused(window),
                    "the key field takes focus"
                );
                v.submit(cx);
                let got = requests.try_recv();
                eprintln!("got ok={} ", got.is_ok());
                assert!(matches!(
                    got,
                    Ok(matrix_engine::signin::SignInCommand::DeferRecovery)
                ));
                v.submit(cx);
                assert!(
                    requests.try_recv().is_err(),
                    "a second activation must not queue another command"
                );
                events
                    .try_send(SignInEvent::Recovery(RecoveryState::Deferred))
                    .unwrap();
                assert_eq!(v.poll(cx), Some(account));
            })
            .unwrap();
            cx.spawn(async move |cx| {
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    fn semantic_prompt_lifecycle(mode: SignInMode) {
        use accesskit::{Action, ActionRequest, TreeId};
        use matrix_desktop::accessibility::{
            NavigationAction, NavigationSnapshot,
            semantic_session::SemanticSession,
            signin::{Control, Intent},
        };
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let view = cx.open_window(gpui::WindowOptions::default(), |_, cx| cx.new(|cx| SignInView::new(SignInUi { events: event_rx, setup: matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap()).1, commands }, cx))).unwrap();
            view.update(cx, |v, window, cx| {
                let nav = |v: &SignInView, window: &Window, cx: &gpui::App| NavigationSnapshot::for_signin(v.semantic_snapshot(window, cx));
                let mut session = SemanticSession::new(nav(v, window, cx));
                assert!(session.tree().nodes.iter().skip(1).all(|(_,n)| n.is_disabled()));
                let mut old = Vec::new();
                for error in [None, Some(SignInError::InvalidHomeserver), Some(SignInError::InvalidUserId), Some(SignInError::InvalidSecret), Some(SignInError::Authentication(matrix_engine::auth::AuthError::CredentialsRejected))] {
                    events.try_send(SignInEvent::Prompt(matrix_engine::signin::SignInPrompt { mode, error })).unwrap();
                    v.poll(cx);
                    let _ = v.render(window, cx);
                    if mode == SignInMode::Unlock {
                        for control in [Control::Homeserver, Control::User, Control::Password] {
                            for focus_only in [true, false] {
                                assert!(!v.apply_semantic(Intent { source: v.form.source().unwrap(), control, focus_only }, window, cx));
                            }
                        }
                    }
                    let expected = if mode == SignInMode::Unlock { Control::Phrase } else { match error { None | Some(SignInError::InvalidHomeserver) => Control::Homeserver, Some(SignInError::InvalidUserId) => Control::User, Some(SignInError::InvalidSecret) => Control::Phrase, _ => Control::Password } };
                    assert_eq!(v.semantic_snapshot(window, cx).focused, Some(expected));
                    session.update(nav(v, window, cx));
                    for (request, intent) in &old {
                        assert!(session.action(request).is_none());
                        assert!(!v.apply_semantic(*intent, window, cx));
                    }
                    for control in Control::ALL.into_iter().filter(|c| !matches!(c, Control::Recover | Control::Defer | Control::Copy) && (mode == SignInMode::Password || matches!(c, Control::Phrase | Control::Submit))) {
                        let id = session.tree().nodes.iter().find(|(_,n)| n.label() == Some(match control {Control::Homeserver => "Homeserver HTTPS", Control::User => "ID Matrix completo", Control::Password => "Password Matrix", Control::Phrase => "Frase locale · almeno 16 caratteri", Control::Submit if mode == SignInMode::Unlock => "Sblocca", Control::Submit => "Accedi", Control::Recover | Control::Defer | Control::Copy => unreachable!()})).unwrap().0;
                        for action in [Action::Focus, Action::Click] {
                            let request = ActionRequest { action, target_tree: TreeId::ROOT, target_node: id, data: None };
                            let Some(NavigationAction::SignIn(intent)) = session.action(&request) else { panic!("current action missing") };
                            if control != Control::Submit || action == Action::Focus {
                                assert!(v.apply_semantic(intent, window, cx));
                            }
                            old.push((request, intent));
                        }
                    }
                    for i in [2,3] { v.fields[i].update(cx, |f,_| { f.input.replace(None,"protected lifecycle sentinel"); }); }
                    session.update(nav(v, window, cx));
                    let dump = format!("{:?}",session.tree());
                    assert!(!dump.contains("protected lifecycle sentinel"));
                    eprintln!("protected semantic update ({mode:?}): {dump}");
                    assert!(session.tree().nodes.iter().filter(|(_,n)| n.role() == accesskit::Role::PasswordInput).all(|(_,n)| n.value().is_none() && n.children().is_empty()));
                }
                // Local validation consumes the source even though it immediately returns a prompt.
                for i in [2,3] { v.fields[i].update(cx, |f,_| f.input = LoginInput::secret()); }
                v.submit(cx);
                session.update(nav(v, window, cx));
                for (request,intent) in &old { assert!(session.action(request).is_none()); assert!(!v.apply_semantic(*intent, window, cx)); }
                let source = v.form.source().unwrap();
                let intent = Intent { source, control: Control::Submit, focus_only: false };
                v.fields[2].update(cx, |f,_| { f.input.mark(None,"protected preedit sentinel",None); });
                assert!(v.apply_semantic(intent, window, cx));
                assert_eq!(v.form.source(), Some(source));
                assert!(requests.try_recv().is_err());
                for i in [2,3] { v.fields[i].update(cx, |f,_| { f.input = LoginInput::secret(); f.input.replace(None,"protected lifecycle sentinel"); }); }
                assert!(v.apply_semantic(intent, window, cx));
                assert_eq!(v.form.status(), FormStatus::Busy);
                assert_eq!(v.focus_next, None, "pending must retire an unpainted error-focus request");
                assert!(!v.apply_semantic(intent, window, cx));
                assert!(requests.try_recv().is_ok()); assert!(requests.try_recv().is_err());
                session.update(nav(v, window, cx));
                assert!(session.tree().nodes.iter().skip(1).all(|(_,n)| n.is_disabled()));
                for event in [
                    SignInEvent::Failed(SignInError::NoSession),
                    SignInEvent::Prompt(matrix_engine::signin::SignInPrompt { mode: SignInMode::Unlock, error: None }),
                    SignInEvent::Prompt(matrix_engine::signin::SignInPrompt { mode: SignInMode::Password, error: None }),
                    SignInEvent::Authenticated(matrix_domain::AccountId::new(9)),
                ] {
                    events.try_send(event).unwrap(); v.poll(cx); session.update(nav(v,window,cx));
                    assert!(!v.apply_semantic(intent,window,cx));
                    for (request,intent) in &old { assert!(session.action(request).is_none()); assert!(!v.apply_semantic(*intent,window,cx)); }
                    for f in &v.fields[2..] { assert!(f.read(cx).input.display_text().is_empty()); }
                    if v.mode == SignInMode::Unlock { assert_eq!(session.tree().nodes.len(),3); }
                    if matches!(event, SignInEvent::Failed(_) | SignInEvent::Authenticated(_)) { assert!(v.fields.iter().all(|f| !f.read(cx).enabled)); }
                    assert!(requests.try_recv().is_err());
                }
                eprintln!("opening, five backend prompt/focus states, local retry, preedit refusal, pending duplicate, failure, unlock, password return and success; retained translated Focus/Click sources retired; protected tree inspection");
            }).unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| { executor.timer(std::time::Duration::from_millis(100)).await; cx.update(|cx| cx.quit()).unwrap(); }).detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn captured_submit_callback_rejects_replaced_prompt_before_consuming_fields() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        eprintln!("probe: create app");
        gpui::Application::new().run(|cx| {
            eprintln!("probe: launch closure");
            let capacity = std::num::NonZeroUsize::new(1).unwrap();
            let (mut events, event_rx) = matrix_engine::bounded(capacity);
            let (commands, mut requests) = matrix_engine::bounded(capacity);
            let prompt = SignInEvent::Prompt(matrix_engine::signin::SignInPrompt {
                mode: SignInMode::Password,
                error: None,
            });
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        SignInView::new(
                            SignInUi {
                                events: event_rx,
                                setup: matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap()).1,
                                commands,
                            },
                            cx,
                        )
                    })
                })
                .unwrap();
            eprintln!("probe: window created");
            view.update(cx, |view, window, cx| {
                assert!(events.try_send(prompt).is_ok());
                view.poll(cx);
                let old = view.submit_callback();
                let old_key = view.key_callback();
                let retired_fields = view.fields.clone();
                assert!(events.try_send(prompt).is_ok());
                view.poll(cx);
                for field in &retired_fields {
                    assert!(
                        !field.read(cx).enabled,
                        "replaced prompts must retire native field entities, not re-enable queued input"
                    );
                }
                for (index, text) in [
                    (2, "fixture-password"),
                    (3, "local fixture phrase long enough"),
                ] {
                    view.fields[index].update(cx, |field, _| field.input.replace(None, text));
                }
                old_key(view, &gpui::KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                old(view, cx);
                assert!(requests.try_recv().is_err());
                assert!(!view.fields[2].read(cx).input.display_text().is_empty());
                let current = view.submit_callback();
                let duplicate = view.submit_callback();
                current(view, cx);
                duplicate(view, cx);
                assert!(matches!(
                    requests.try_recv(),
                    Ok(matrix_engine::signin::SignInCommand::Password { .. })
                ));
                assert!(requests.try_recv().is_err());
                assert_eq!(view.form.status(), FormStatus::Busy);
                view.fields[2].update(cx, |field, _| {
                    field.input.replace(None, "obsolete fixture password")
                });
                view.fields[3].update(cx, |field, _| {
                    field.input.replace(None, "obsolete local fixture phrase")
                });
                assert!(
                    events
                        .try_send(SignInEvent::Prompt(matrix_engine::signin::SignInPrompt {
                            mode: SignInMode::Unlock,
                            error: None,
                        }))
                        .is_ok()
                );
                view.poll(cx);
                assert!(
                    !view.fields[0].read(cx).enabled,
                    "hidden password-form controls must be disabled in Unlock"
                );
                assert!(!view.fields[1].read(cx).enabled);
                assert!(!view.fields[2].read(cx).enabled);
                assert!(view.fields[3].read(cx).enabled);
                assert!(view.fields[2].read(cx).input.display_text().is_empty());
                assert!(view.fields[3].read(cx).input.display_text().is_empty());
                let unlock = view.submit_callback();
                view.fields[3].update(cx, |field, _| {
                    field.input.mark(None, "fixture preedit", None)
                });
                unlock(view, cx);
                assert!(requests.try_recv().is_err());
                assert!(view.fields[3].read(cx).input.marked_utf16().is_some());
                view.fields[3].update(cx, |field, _| field.input = LoginInput::secret());
                let account = matrix_domain::AccountId::new(7);
                view.fields[3].update(cx, |field, _| {
                    field.input.replace(None, "fixture phrase before success")
                });
                assert!(events.try_send(SignInEvent::Authenticated(account)).is_ok());
                assert_eq!(view.poll(cx), Some(account));
                unlock(view, cx);
                assert!(requests.try_recv().is_err());
                assert!(
                    view.fields[3].read(cx).input.display_text().is_empty(),
                    "terminal event must clear painted secret field too"
                );
            })
            .unwrap();
            eprintln!("probe: assertions complete");
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(100)).await;
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }
}
