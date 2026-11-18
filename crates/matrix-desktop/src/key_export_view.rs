//! Sub-pages of «Privacy e sicurezza» inside the drawer: «Esporta chiavi» and «Chiave di recupero».
//! State and texts live in `matrix_desktop::key_export`; the engine does the work.
use crate::login_field::LoginField;
use crate::*;
use gpui::{Entity, MouseButton};
use matrix_desktop::key_export::*;

const EXPORT_FILE_NAME: &str = "chiavi-gnobbi.txt";

/// Widgets of the open sub-page. The typed secrets live only in the fields, which wipe on take.
pub(crate) struct KeysUi {
    /// Legacy profiles: [phrase, password, repeat]; vault profiles and «Rigenera»: [password, repeat] / [].
    fields: Vec<Entity<LoginField>>,
    legacy: bool,
    button_focus: FocusHandle,
    copy_focus: FocusHandle,
    stop: usize,
    selection: Rc<RefCell<matrix_desktop::static_selection::StaticSelection>>,
    /// Bumped by every copy; only the timer of the latest one may wipe the clipboard.
    copy_gen: u64,
}

impl Shell {
    /// Rows 7 and 8 exist only with a live session.
    pub(crate) fn keys_rows(&self) -> bool {
        self.connected
    }

    /// A legacy profile (no `protection-mode` file in its root) asks for the local unlock phrase again.
    fn keys_need_phrase(&self) -> bool {
        self.account_root
            .as_ref()
            .is_some_and(|(root, _)| !root.join("protection-mode").exists())
    }

    pub(crate) fn open_keys_row(
        &mut self,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_row = row;
        let kind = if row == matrix_desktop::SETTINGS_EXPORT_KEYS_ROW {
            KeysKind::Export
        } else {
            KeysKind::Reset
        };
        let legacy = self.keys_need_phrase();
        let mut fields = Vec::new();
        let mut add = |placeholder: &'static str, cx: &mut Context<Self>| {
            let field = cx.new(|cx| {
                let mut field = LoginField::new(true, cx);
                field.enabled = true;
                field.placeholder = placeholder;
                field
            });
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            fields.push(field);
        };
        if legacy {
            add("Frase di sblocco", cx);
        }
        if kind == KeysKind::Export {
            add("Password", cx);
            add("Ripeti password", cx);
        }
        self.verification.keys = Some(KeysPage::new(kind));
        let ui = KeysUi {
            fields,
            legacy,
            button_focus: cx.focus_handle(),
            copy_focus: cx.focus_handle(),
            stop: 0,
            selection: Default::default(),
            copy_gen: 0,
        };
        self.keys_ui = Some(ui);
        self.focus_keys_stop(0, window, cx);
        cx.notify();
    }

    /// Esc, the header and «‹»: refused while the new key is shown or being made.
    pub(crate) fn close_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.verification.keys.as_ref() else {
            return;
        };
        if !page.closable() {
            return;
        }
        let row = if page.kind == KeysKind::Export {
            matrix_desktop::SETTINGS_EXPORT_KEYS_ROW
        } else {
            matrix_desktop::SETTINGS_RESET_RECOVERY_ROW
        };
        self.verification.keys = None;
        self.keys_ui = None;
        self.settings_row = row;
        self.settings_focus.focus(window);
        cx.notify();
    }

    fn focus_keys_stop(&mut self, stop: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ui) = self.keys_ui.as_mut() else {
            return;
        };
        let shown = self.verification.keys.as_ref().is_some_and(|p| p.shown());
        let count = if shown { 2 } else { ui.fields.len() + 1 };
        let stop = stop % count;
        ui.stop = stop;
        if shown {
            [&ui.copy_focus, &ui.button_focus][stop].focus(window);
        } else if let Some(field) = ui.fields.get(stop) {
            field.read(cx).focus.clone().focus(window);
        } else {
            ui.button_focus.focus(window);
        }
        cx.notify();
    }

    /// The stop that holds the focus now (a click may have moved it), else the last one used.
    fn current_keys_stop(&self, window: &Window, cx: &Context<Self>) -> usize {
        let Some(ui) = &self.keys_ui else { return 0 };
        let shown = self.verification.keys.as_ref().is_some_and(|p| p.shown());
        let mut handles: Vec<FocusHandle> = if shown {
            vec![ui.copy_focus.clone()]
        } else {
            ui.fields.iter().map(|f| f.read(cx).focus.clone()).collect()
        };
        handles.push(ui.button_focus.clone());
        handles
            .iter()
            .position(|h| h.is_focused(window))
            .unwrap_or(ui.stop)
    }

    /// Runs from `menu_key` while a sub-page is open; true when the key was consumed. Space and
    /// ordinary keys go to the focused field.
    pub(crate) fn keys_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = &event.keystroke;
        let m = &key.modifiers;
        let command = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        let shown = self.verification.keys.as_ref().is_some_and(|p| p.shown());
        let stop = self.current_keys_stop(window, cx);
        match key.key.as_str() {
            "escape" => self.close_keys(window, cx),
            "tab" | "up" | "down" => {
                let count = if shown {
                    2
                } else {
                    self.keys_ui.as_ref().map_or(1, |u| u.fields.len() + 1)
                };
                let back = key.key == "up" || m.shift;
                self.focus_keys_stop(
                    (stop + if back { count - 1 } else { 1 }) % count,
                    window,
                    cx,
                );
            }
            "enter" if !m.modified() => match (shown, stop) {
                (true, 0) => self.copy_recovery_key(None, cx),
                (true, _) => self.confirm_key_saved(window, cx),
                _ => self.submit_keys(window, cx),
            },
            "c" if command && shown => {
                let text = self.keys_selected_text();
                match text {
                    Some(text) => self.copy_recovery_key(Some(text), cx),
                    None => return false,
                }
            }
            "a" if command && shown => {
                if let (Some(ui), Some(page)) = (&self.keys_ui, &self.verification.keys)
                    && let Some(secret) = &page.secret
                {
                    ui.selection.borrow_mut().select_all(secret.expose());
                    cx.notify();
                }
            }
            _ => return false,
        }
        true
    }

    fn keys_selected_text(&self) -> Option<String> {
        let (ui, page) = (self.keys_ui.as_ref()?, self.verification.keys.as_ref()?);
        let secret = page.secret.as_ref()?;
        let text = ui.selection.borrow().copy(secret.expose())?.to_owned();
        Some(text)
    }

    pub(crate) fn submit_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.verification.keys.as_ref().map(|p| (p.kind, p.busy)) {
            Some((_, true)) | None => {}
            Some((KeysKind::Export, false)) => self.submit_keys_export(cx),
            Some((KeysKind::Reset, false)) => self.submit_keys_reset(cx),
        }
        let _ = window;
    }

    /// Puts the typed values back (after a refusal or a failed try) so the user can retry at once.
    fn fill_keys_fields(
        &self,
        phrase: Option<&str>,
        password: &str,
        repeat: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(ui) = &self.keys_ui else { return };
        let mut values: Vec<&str> = Vec::new();
        values.extend(phrase.filter(|_| ui.legacy));
        values.extend([password, repeat]);
        // Legacy without a phrase value keeps its first field empty.
        let skip = usize::from(ui.legacy && phrase.is_none());
        for (field, value) in ui.fields.iter().skip(skip).zip(values) {
            field.update(cx, |f, _| {
                drop(f.input.take());
                f.input.replace(None, value);
            });
        }
    }

    fn submit_keys_export(&mut self, cx: &mut Context<Self>) {
        let Some(ui) = &self.keys_ui else { return };
        let legacy = ui.legacy;
        let take =
            |i: usize, cx: &mut Context<Self>| ui.fields[i].update(cx, |f, _| f.input.take());
        let phrase = legacy.then(|| take(0, cx));
        let (password, repeat) = (
            take(usize::from(legacy), cx),
            take(usize::from(legacy) + 1, cx),
        );
        let refusal = if legacy && phrase.as_ref().is_some_and(|p| p.is_empty()) {
            Some(WRONG_PHRASE)
        } else {
            check_password(&password, &repeat)
                .err()
                .map(PasswordError::text)
        };
        if let Some(text) = refusal {
            self.fill_keys_fields(
                phrase.as_deref().map(String::as_str),
                &password,
                &repeat,
                cx,
            );
            if let Some(page) = self.verification.keys.as_mut() {
                page.line = Some((text.into(), true));
            }
            cx.notify();
            return;
        }
        if let Some(page) = self.verification.keys.as_mut() {
            page.busy = true;
            page.line = None;
        }
        let picked =
            cx.prompt_for_new_path(&matrix_desktop::save_start_dir(), Some(EXPORT_FILE_NAME));
        cx.spawn(async move |this, cx| {
            // No portal or a cancelled picker: nothing happens and the dialog stays as it was.
            let path = picked.await.ok().and_then(|r| r.ok()).flatten();
            let _ = this.update(cx, |this, cx| {
                let queued = path.is_some_and(|dest| {
                    this.verification
                        .export_keys(dest, &password, phrase.as_ref())
                });
                // Queued: the command carries the secrets and the page keeps one wiped-on-drop copy for a retry.
                if !queued {
                    if let Some(page) = this.verification.keys.as_mut() {
                        page.busy = false;
                    }
                    this.fill_keys_fields(
                        phrase.as_deref().map(String::as_str),
                        &password,
                        &repeat,
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn submit_keys_reset(&mut self, cx: &mut Context<Self>) {
        let Some(ui) = &self.keys_ui else { return };
        let phrase = ui
            .legacy
            .then(|| ui.fields[0].update(cx, |f, _| f.input.take()));
        self.verification.reset_recovery(phrase.as_ref());
        cx.notify();
    }

    /// After an engine answer: refill the fields of a failed export (the unlock phrase only when it was right), once.
    pub(crate) fn sync_keys_fields(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.verification.keys.as_mut() else {
            return;
        };
        if !std::mem::take(&mut page.refill) {
            return;
        }
        let wrong_phrase = page
            .line
            .as_ref()
            .is_some_and(|(text, _)| text == WRONG_PHRASE);
        if let Some(held) = page.held.take() {
            let phrase = held.phrase.as_ref().filter(|_| !wrong_phrase);
            self.fill_keys_fields(
                phrase.map(|p| p.as_str()),
                &held.password,
                &held.password,
                cx,
            );
        }
        cx.notify();
    }

    /// «L'ho salvata»: clipboard wiped if it still holds the key, then the key is dropped and zeroized.
    pub(crate) fn confirm_key_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wipe_key_clipboard(cx);
        self.verification.confirm_key_saved();
        self.keys_ui = None;
        self.settings_row = matrix_desktop::SETTINGS_RESET_RECOVERY_ROW;
        self.settings_focus.focus(window);
        cx.notify();
    }

    /// Copies the whole key (icon) or the selected part (Cmd/Ctrl+C) and arms the clipboard timer.
    pub(crate) fn copy_recovery_key(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        let Some(secret) = self
            .verification
            .keys
            .as_ref()
            .and_then(|p| p.secret.as_ref())
        else {
            return;
        };
        let text = text.unwrap_or_else(|| secret.expose().to_owned());
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let Some(ui) = self.keys_ui.as_mut() else {
            return;
        };
        ui.copy_gen += 1;
        let generation = ui.copy_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(CLIPBOARD_SECONDS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .keys_ui
                    .as_ref()
                    .is_some_and(|u| u.copy_gen == generation)
                {
                    this.wipe_key_clipboard(cx);
                }
            });
        })
        .detach();
    }

    /// Empties the clipboard only if it still holds the key (or the part of it selected here): never someone else's copy.
    // shortcut: a clipboard-history manager of the OS may already hold a copy; we cannot reach it.
    fn wipe_key_clipboard(&self, cx: &mut Context<Self>) {
        let (Some(ui), Some(secret)) = (
            &self.keys_ui,
            self.verification
                .keys
                .as_ref()
                .and_then(|p| p.secret.as_ref()),
        ) else {
            return;
        };
        let Some(clip) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let part = ui
            .selection
            .borrow()
            .copy(secret.expose())
            .map(str::to_owned);
        if clip == secret.expose() || part.as_deref() == Some(clip.as_str()) {
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        }
    }

    fn keys_button(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        focused: bool,
        t: matrix_desktop::DesignTokens,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .mx_4()
            .my_2()
            .h(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(rgb(if focused { t.text } else { t.accent }))
            .text_color(rgb(if enabled { t.accent } else { t.disabled }))
            .when(enabled, |d| d.cursor_pointer())
            .child(label)
    }

    /// The whole sub-page; `None` when no page is open.
    pub(crate) fn render_keys_page(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (ui, page) = (self.keys_ui.as_ref()?, self.verification.keys.as_ref()?);
        let copy_ring = input_modality::only_keys(ui.copy_focus.is_focused(window), cx);
        let button_ring = input_modality::only_keys(ui.button_focus.is_focused(window), cx);
        let title = if page.kind == KeysKind::Export {
            "‹ Esporta chiavi"
        } else {
            "‹ Chiave di recupero"
        };
        let mut body = div().child(
            div()
                .id("keys-back")
                .h(px(t.header))
                .px_4()
                .flex()
                .items_center()
                .cursor_pointer()
                .text_color(rgb(t.text))
                .child(title)
                .on_click(cx.listener(|this, _, window, cx| this.close_keys(window, cx))),
        );
        let line = |text: &str, error: bool| {
            div()
                .mx_4()
                .text_size(px(12.))
                .text_color(rgb(if error { t.error } else { t.secondary }))
                .child(text.to_owned())
        };
        if let Some(secret) = page
            .secret
            .as_ref()
            .filter(|_| page.kind == KeysKind::Reset)
        {
            let text = secret.expose().to_owned();
            let range = ui.selection.borrow().range(&text);
            let styled = gpui::StyledText::new(text).with_highlights(range.into_iter().map(|r| {
                (
                    r,
                    gpui::HighlightStyle {
                        background_color: Some(rgb(t.selection).into()),
                        ..Default::default()
                    },
                )
            }));
            let (down, drag) = (styled.layout().clone(), styled.layout().clone());
            body = body
                .child(
                    div()
                        .mx_4()
                        .my_2()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .id("recovery-key")
                                .flex_1()
                                .min_w_0()
                                .p_3()
                                .border_2()
                                .border_color(rgb(t.accent))
                                .font_family(matrix_desktop::MONO_FONT)
                                .text_size(px(15.))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                                        this.keys_selection_press(
                                            &down,
                                            e.position,
                                            e.click_count,
                                            cx,
                                        )
                                    }),
                                )
                                .on_mouse_move(cx.listener(
                                    move |this, e: &gpui::MouseMoveEvent, _, cx| {
                                        if e.pressed_button == Some(MouseButton::Left) {
                                            this.keys_selection_drag(&drag, e.position, cx)
                                        }
                                    },
                                ))
                                .child(styled),
                        )
                        // Icon only, same box as the room-id copy; the label is for screen readers.
                        .child(
                            div()
                                .id("copy-recovery-key")
                                .track_focus(&ui.copy_focus)
                                .flex_shrink_0()
                                .w(px(28.))
                                .h(px(28.))
                                .mt_3()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_lg()
                                .border_1()
                                .border_color(if copy_ring {
                                    rgb(t.accent)
                                } else {
                                    gpui::rgba(0)
                                })
                                .hover(|s| s.bg(rgb(t.border)))
                                .cursor_pointer()
                                .child(composer_icons::icon(composer_icons::Icon::Copy, t.accent))
                                .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                                    if !event.is_keyboard() {
                                        this.copy_recovery_key(None, cx)
                                    }
                                })),
                        ),
                )
                .child(line(SAVE_WARNING, false))
                .child(
                    self.keys_button("keys-saved", "L'ho salvata", true, button_ring, t)
                        .track_focus(&ui.button_focus)
                        .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                            if !event.is_keyboard() {
                                this.confirm_key_saved(window, cx)
                            }
                        })),
                );
            return Some(body.into_any_element());
        }
        for field in &ui.fields {
            field.update(cx, |f, _| {
                f.theme = t;
                f.enabled = !page.busy;
            });
            body = body.child(div().mx_4().my_1().child(field.clone()));
        }
        let (label, submitted) = if page.kind == KeysKind::Export {
            ("Esporta…", "keys-export")
        } else {
            ("Rigenera", "keys-reset")
        };
        if page.kind == KeysKind::Reset {
            body = body.child(line(RESET_WARNING, false));
        }
        body = body.child(
            self.keys_button(submitted, label, !page.busy, button_ring, t)
                .track_focus(&ui.button_focus)
                .when(!page.busy, |d| {
                    d.on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                        // Enter is handled in `keys_key`; the key-up click of the focused button would fire twice.
                        if !event.is_keyboard() {
                            this.submit_keys(window, cx)
                        }
                    }))
                }),
        );
        if page.kind == KeysKind::Reset && page.busy {
            body = body.child(line(RESET_RUNNING, false));
        } else if let Some((text, error)) = &page.line {
            body = body.child(line(text, *error));
        }
        Some(body.into_any_element())
    }

    fn keys_selection_press(
        &mut self,
        layout: &gpui::TextLayout,
        at: gpui::Point<gpui::Pixels>,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        if let (Some(ui), Some(secret)) = (
            &self.keys_ui,
            self.verification
                .keys
                .as_ref()
                .and_then(|p| p.secret.as_ref()),
        ) {
            let index = layout.index_for_position(at).unwrap_or_else(|i| i);
            ui.selection
                .borrow_mut()
                .press(secret.expose(), index, clicks);
            cx.notify();
        }
    }

    fn keys_selection_drag(
        &mut self,
        layout: &gpui::TextLayout,
        at: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let (Some(ui), Some(secret)) = (
            &self.keys_ui,
            self.verification
                .keys
                .as_ref()
                .and_then(|p| p.secret.as_ref()),
        ) {
            let index = layout.index_for_position(at).unwrap_or_else(|i| i);
            ui.selection.borrow_mut().drag(secret.expose(), index);
            cx.notify();
        }
    }
}
