//! «Indirizzi» in the room details sheet and the «Aggiungi indirizzo» dialog.
//! The engine does the work (`alias_ops`); every answer carries the list re-read from the server.
use crate::switch_row::switch_row;
use crate::*;
use matrix_engine::alias::{AliasAction, AliasOpError, RoomAliases};
use matrix_engine::operations::OperationCommand;

pub(crate) struct AliasPanel {
    scope: RoomScope,
    data: Option<RoomAliases>,
    /// A load is out or failed: it is not asked again until an operation answers without a list.
    requested: bool,
    busy: bool,
    error: Option<AliasOpError>,
}

pub(crate) struct AliasAdd {
    scope: RoomScope,
    field: gpui::Entity<crate::login_field::LoginField>,
    server: String,
    /// Asked only when the room has no main address yet.
    ask_main: bool,
    make_main: bool,
    busy: bool,
    error: Option<AliasOpError>,
    main_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
}

pub(crate) fn error_text(error: &AliasOpError) -> &'static str {
    match error {
        AliasOpError::AliasInUse => "Indirizzo già in uso",
        AliasOpError::Forbidden => "Non hai il permesso",
        AliasOpError::InvalidName => "Nome non valido",
        AliasOpError::Network => "Connessione non riuscita",
        AliasOpError::ServerUnreachable => "Il server non risponde",
        AliasOpError::Orphaned { .. } => "Indirizzo creato ma non collegato",
        AliasOpError::WrongScope | AliasOpError::Unavailable => "Stanza non disponibile",
    }
}

/// Main first, then the other published ones, then the local ones not published.
fn rows(data: &RoomAliases) -> Vec<(String, bool)> {
    let c = &data.canonical;
    let mut all: Vec<(String, bool)> = c.main.iter().map(|a| (a.clone(), true)).collect();
    for a in c.alts.iter().chain(&data.local) {
        if !all.iter().any(|(x, _)| x == a) {
            all.push((a.clone(), false));
        }
    }
    all
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cmd {
    Main,
    Copy,
    Remove,
    Add,
}

/// Every command of the block in screen order, with its screen-reader label; the keyboard ring, the tree and the view all read this one list.
fn commands(data: &RoomAliases) -> Vec<(Cmd, String, String)> {
    let (edit, own) = (data.can_edit_aliases, format!(":{}", data.server));
    let list = rows(data);
    let mut out = vec![];
    for (alias, main) in &list {
        if edit && !main {
            out.push((Cmd::Main, alias.clone(), format!("Imposta principale {alias}")));
        }
        out.push((Cmd::Copy, alias.clone(), format!("Copia {alias}")));
        // An address of another server cannot be deleted from here.
        if edit && alias.ends_with(&own) {
            out.push((Cmd::Remove, alias.clone(), format!("Rimuovi {alias}")));
        }
    }
    if edit {
        out.push((Cmd::Add, String::new(), "Aggiungi indirizzo".into()));
    }
    out
}

impl AliasAdd {
    /// The local part as typed (surrounding space ignored) and the full address, if valid.
    fn preview(&self, cx: &gpui::App) -> Option<String> {
        matrix_engine::alias::compose_alias(self.field.read(cx).input.display_text().trim(), &self.server).ok()
    }

    fn typed(&self, cx: &gpui::App) -> bool {
        !self.field.read(cx).input.display_text().trim().is_empty()
    }
}

impl Shell {
    /// Asks for the list of the shown room once; called with the other pending operations.
    pub(crate) fn ensure_aliases(&mut self) {
        let scope = self.sidebar.selected().filter(|_| self.room_details_open).map(|r| r.scope.clone());
        let Some(scope) = scope else {
            self.aliases = None;
            return;
        };
        if self.aliases.as_ref().is_none_or(|p| p.scope != scope) {
            self.aliases = Some(AliasPanel { scope: scope.clone(), data: None, requested: false, busy: false, error: None });
        }
        if self.pending_operation_command.is_none()
            && let Some(panel) = self.aliases.as_mut().filter(|p| !p.requested)
        {
            panel.requested = true;
            self.pending_operation_command = Some(OperationCommand::Aliases { scope, action: AliasAction::Load });
        }
    }

    fn send_alias(&mut self, action: AliasAction, cx: &mut Context<Self>) -> bool {
        let Some(panel) = self.aliases.as_mut().filter(|p| !p.busy) else { return false };
        if self.pending_operation_command.is_some() {
            return false;
        }
        panel.busy = true;
        panel.error = None;
        self.pending_operation_command = Some(OperationCommand::Aliases { scope: panel.scope.clone(), action });
        cx.notify();
        true
    }

    pub(crate) fn aliases_answered(
        &mut self,
        scope: RoomScope,
        action: AliasAction,
        result: Result<(), AliasOpError>,
        aliases: Option<RoomAliases>,
        cx: &mut Context<Self>,
    ) {
        if let Some(add) = self.alias_add.as_mut().filter(|a| a.scope == scope && a.busy && matches!(action, AliasAction::Add { .. })) {
            add.busy = false;
            match &result {
                Ok(()) => self.alias_add = None,
                Err(e) => {
                    add.error = Some(e.clone());
                    add.field.update(cx, |f, _| f.enabled = true);
                }
            }
        }
        if let Some(panel) = self.aliases.as_mut().filter(|p| p.scope == scope) {
            panel.busy = false;
            // Without a fresh list after a change the next frame asks again.
            panel.requested = !(aliases.is_none() && result.is_ok());
            panel.data = aliases;
            if !matches!(action, AliasAction::Add { .. }) {
                panel.error = result.err();
            }
        }
        cx.notify();
    }

    /// Commands in the Tab ring: none while the add dialog is open.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn alias_commands(&self) -> Vec<(Cmd, String, String)> {
        match (&self.aliases, &self.alias_add) {
            (Some(AliasPanel { data: Some(d), .. }), None) => commands(d),
            _ => vec![],
        }
    }

    /// Return/Space/click of the `index`th command; true when the add dialog took the focus.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn alias_act(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((cmd, alias, _)) = self.alias_commands().into_iter().nth(index) else { return false };
        match cmd {
            Cmd::Main => _ = self.send_alias(AliasAction::SetCanonical { alias }, cx),
            Cmd::Copy => cx.write_to_clipboard(gpui::ClipboardItem::new_string(alias)),
            Cmd::Remove => _ = self.send_alias(AliasAction::Remove { alias }, cx),
            Cmd::Add => {
                self.open_alias_add(window, cx);
                return self.alias_add.is_some();
            }
        }
        false
    }

    pub(crate) fn open_alias_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(data) = self.aliases.as_ref().and_then(|p| p.data.clone().filter(|d| d.can_edit_aliases).map(|d| (p.scope.clone(), d))) else { return };
        let (scope, data) = data;
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field.placeholder = "nome";
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field.read(cx).focus.focus(window);
        let ask_main = data.canonical.main.is_none();
        self.alias_add = Some(AliasAdd {
            scope,
            field,
            server: data.server,
            ask_main,
            make_main: true,
            busy: false,
            error: None,
            main_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
        });
        cx.notify();
    }

    fn close_alias_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.alias_add = None;
        self.list_focus.focus(window);
        cx.notify();
    }

    fn submit_alias_add(&mut self, cx: &mut Context<Self>) {
        let Some(add) = self.alias_add.as_mut().filter(|a| !a.busy) else { return };
        if add.preview(cx).is_none() {
            return;
        }
        let local = add.field.read(cx).input.display_text().trim().to_owned();
        let make_canonical = add.ask_main.then_some(add.make_main);
        let scope = add.scope.clone();
        if self.aliases.as_ref().is_some_and(|p| p.scope == scope) && self.send_alias(AliasAction::Add { local, make_canonical }, cx) {
            let Some(add) = self.alias_add.as_mut() else { return };
            add.busy = true;
            add.error = None;
            add.field.update(cx, |f, _| f.enabled = false);
        }
    }

    fn alias_add_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(add) = &self.alias_add else { return };
        let key = event.keystroke.key.as_str();
        if key == "tab" {
            let mut ring = vec![add.field.read(cx).focus.clone()];
            if add.ask_main {
                ring.push(add.main_focus.clone());
            }
            ring.extend([add.cancel_focus.clone(), add.submit_focus.clone()]);
            let at = ring.iter().position(|h| h.is_focused(window)).unwrap_or(0);
            let next = if event.keystroke.modifiers.shift { (at + ring.len() - 1) % ring.len() } else { (at + 1) % ring.len() };
            ring[next].focus(window);
        } else if key == "escape" || (matches!(key, "enter" | "space") && add.cancel_focus.is_focused(window)) {
            self.close_alias_add(window, cx);
        } else if key == "space" && add.main_focus.is_focused(window) {
            if let Some(add) = self.alias_add.as_mut().filter(|a| !a.busy) {
                add.make_main = !add.make_main;
            }
        } else if key == "enter" || (key == "space" && add.submit_focus.is_focused(window)) {
            self.submit_alias_add(cx);
        } else {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// The «Indirizzi» block of the sheet: the list is always shown, the commands only with the right to edit.
    pub(crate) fn render_aliases(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let panel = self.aliases.as_ref()?;
        let data = panel.data.as_ref()?;
        let list = rows(data);
        let edit = data.can_edit_aliases;
        if list.is_empty() && !edit {
            return None;
        }
        let cmds = commands(data);
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        let focused = self.alias_focus;
        #[cfg(not(all(feature = "linux-a11y", target_os = "linux")))]
        let focused: Option<usize> = None;
        // Icon only: the label is for the screen reader (tree); same 32x32 box and keyboard-only ring as «Copia ID».
        let button = |cmd: Cmd, alias: &str, icon: composer_icons::Icon, cx: &mut Context<Self>| {
            let at = cmds.iter().position(|(c, a, _)| *c == cmd && a == alias);
            div().id(("alias-cmd", at.unwrap_or_default())).relative().children(at.and_then(|i| self.alias_observer(i, cx))).flex_shrink_0().w(px(32.)).h(px(32.)).flex().items_center().justify_center().rounded_lg()
                .border_1().border_color(if input_modality::only_keys(at.is_some() && focused == at, cx) { rgb(t.accent) } else { gpui::rgba(0) })
                .hover(|s| s.bg(rgb(t.border))).cursor_pointer().child(composer_icons::icon(icon, t.accent))
        };
        let own = format!(":{}", data.server);
        Some(
            div()
                .flex()
                .flex_col()
                .text_size(px(12.))
                .text_color(rgb(t.secondary))
                .child("Indirizzi")
                .children(list.into_iter().map(|(alias, main)| {
                    let (copy, set, remove) = (alias.clone(), alias.clone(), alias.clone());
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div().flex_1().min_w_0().truncate().text_size(px(14.))
                                .text_color(rgb(if main { t.accent } else { t.text }))
                                .when(main, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                .child(alias.clone()),
                        )
                        .when(edit && !main, |d| d.child(button(Cmd::Main, &alias, composer_icons::Icon::Check, cx).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.send_alias(AliasAction::SetCanonical { alias: set.clone() }, cx);
                            },
                        ))))
                        .child(button(Cmd::Copy, &alias, composer_icons::Icon::Copy, cx).on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()));
                        })))
                        .when(edit && alias.ends_with(&own), |d| d.child(button(Cmd::Remove, &alias, composer_icons::Icon::Close, cx).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.send_alias(AliasAction::Remove { alias: remove.clone() }, cx);
                            },
                        ))))
                }))
                .children(panel.error.as_ref().map(|e| div().text_color(rgb(t.error)).child(error_text(e))))
                .when(edit, |d| {
                    let at = cmds.iter().position(|(c, ..)| *c == Cmd::Add);
                    d.child(
                        div().id("alias-add-open").relative().children(at.and_then(|i| self.alias_observer(i, cx))).text_size(px(14.)).py_1().cursor_pointer().text_color(rgb(t.accent))
                            .border_1().border_color(if input_modality::only_keys(at.is_some() && focused == at, cx) { rgb(t.accent) } else { gpui::rgba(0) })
                            .child("Aggiungi indirizzo")
                            .on_click(cx.listener(|this, _, window, cx| this.open_alias_add(window, cx))),
                    )
                })
                .into_any_element(),
        )
    }

    pub(crate) fn render_alias_add(&self, t: matrix_desktop::DesignTokens, window: &Window, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        use gpui::{Animation, AnimationExt};
        let add = self.alias_add.as_ref()?;
        add.field.update(cx, |f, _| f.theme = t);
        let preview = add.preview(cx);
        // Live check: a typed name the engine refuses shows its error before any request.
        let shown = add.error.as_ref().map(error_text).or_else(|| (add.typed(cx) && preview.is_none()).then_some("Nome non valido"));
        let orphan = matches!(add.error, Some(AliasOpError::Orphaned { .. }));
        let ready = preview.is_some() && !add.busy;
        let button = |id: &'static str, label: &'static str, color: u32, focus: &FocusHandle| {
            div().id(id).track_focus(focus).px_4().py_2().rounded_md().cursor_pointer().border_1()
                .border_color(rgb(if input_modality::only_keys(focus.is_focused(window), cx) { t.accent } else { t.surface }))
                .text_color(rgb(color)).child(label)
        };
        Some(
            div()
                .id("alias-add-layer")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_alias_add(window, cx)))
                .child(
                    div()
                        .id("alias-add-dialog").w(px(320.)).p_4().gap_2().flex().flex_col().rounded_lg()
                        .bg(rgb(t.surface)).border_1().border_color(rgb(t.border))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.alias_add_key(event, window, cx)))
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.text)).child("Aggiungi indirizzo"))
                        .child(
                            div().flex().items_center().gap_1()
                                .child(div().text_color(rgb(t.secondary)).child("#"))
                                .child(div().flex_1().min_w_0().child(add.field.clone()))
                                .child(div().flex_shrink_0().text_color(rgb(t.secondary)).child(format!(":{}", add.server))),
                        )
                        .children(preview.map(|p| div().text_size(px(13.)).text_color(rgb(t.secondary)).truncate().child(p)))
                        .when(add.ask_main, |d| d.child(
                            switch_row("alias-add-main", "Indirizzo principale", add.make_main, !add.busy,
                                input_modality::only_keys(add.main_focus.is_focused(window), cx), t)
                                .track_focus(&add.main_focus)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(a) = this.alias_add.as_mut().filter(|a| !a.busy) {
                                        a.make_main = !a.make_main;
                                    }
                                    cx.notify();
                                })),
                        ))
                        .children(shown.map(|e| div().text_size(px(13.)).text_color(rgb(t.error)).child(e)))
                        .child(
                            div().flex().justify_end().items_center().gap_2()
                                .when(add.busy, |d| d.child(
                                    div().size(px(8.)).flex_shrink_0().rounded_full().bg(rgb(t.accent)).with_animation(
                                        "alias-add-dot",
                                        Animation::new(std::time::Duration::from_secs(1)).repeat(),
                                        |d, delta| d.opacity(gpui::pulsating_between(0.3, 1.0)(delta)),
                                    ),
                                ))
                                .child(button("alias-add-cancel", "Annulla", t.accent, &add.cancel_focus)
                                    .on_click(cx.listener(|this, _, window, cx| this.close_alias_add(window, cx))))
                                .child(button("alias-add-submit", if orphan { "Riprova" } else { "Aggiungi" }, if ready { t.accent } else { t.disabled }, &add.submit_focus)
                                    .on_click(cx.listener(|this, _, _, cx| this.submit_alias_add(cx)))),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::alias::Canonical;

    #[test]
    fn rows_put_main_first_and_dedupe() {
        let data = RoomAliases {
            canonical: Canonical { main: Some("#a:s".into()), alts: vec!["#b:s".into()] },
            local: vec!["#b:s".into(), "#c:s".into(), "#a:s".into()],
            can_edit_aliases: false,
            server: "s".into(),
        };
        let r = rows(&data);
        assert_eq!(r, vec![("#a:s".into(), true), ("#b:s".into(), false), ("#c:s".into(), false)]);
    }

    #[test]
    fn every_engine_error_has_one_line() {
        assert_eq!(error_text(&AliasOpError::AliasInUse), "Indirizzo già in uso");
        assert_eq!(error_text(&AliasOpError::Forbidden), "Non hai il permesso");
        assert_eq!(error_text(&AliasOpError::InvalidName), "Nome non valido");
        assert_ne!(error_text(&AliasOpError::Network), error_text(&AliasOpError::ServerUnreachable));
        assert_eq!(error_text(&AliasOpError::Orphaned { alias: "#a:s".into() }), "Indirizzo creato ma non collegato");
    }
}
