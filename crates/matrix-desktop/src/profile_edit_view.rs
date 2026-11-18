//! «Modifica profilo» from the account drawer: display name and avatar of the signed-in account.
//! Same dialog shape as «Modifica stanza». `submit_profile_edit` queues `SaveProfile` on the verification
//! route (it holds the SDK client) and `profile_saved` takes the `ProfileSaved` answer.
use crate::*;
use matrix_engine::operations::OperationError;
use std::sync::Arc;

pub(crate) use matrix_engine::verification::ProfileAvatar as AvatarChange;

/// What the dialog asks the engine to save; `name` is None when it did not change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProfileDraft {
    pub name: Option<String>,
    pub avatar: AvatarChange,
}

pub(crate) struct ProfileEdit {
    pub(crate) field: gpui::Entity<crate::login_field::LoginField>,
    original: String,
    initials: String,
    /// Picture the account has now (already decoded for the drawer), None = initials.
    current: Option<Arc<gpui::RenderImage>>,
    avatar: AvatarChange,
    /// The chosen file, square-cropped by the engine's own pipeline (never decoded by GPUI itself).
    preview: Option<Arc<gpui::RenderImage>>,
    /// Why the chosen file was refused; a fixed text, never the path.
    file_error: Option<&'static str>,
    /// The server's capabilities forbid the change: controls disabled, one error line.
    allowed: bool,
    avatar_focus: FocusHandle,
    remove_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    busy: bool,
    error: Option<OperationError>,
}

/// Bounded read + square crop (`prepare_blocking`), then the RGBA preview as a GPUI image.
fn load_preview(path: &std::path::Path) -> Result<Arc<gpui::RenderImage>, &'static str> {
    #[cfg(feature = "live")]
    {
        let picture = matrix_engine::profile::prepare_blocking(path).map_err(|e| e.message())?;
        preview_image(picture.preview).ok_or("Scegli un'immagine PNG, JPEG o WebP valida.")
    }
    #[cfg(not(feature = "live"))]
    {
        let _ = path;
        Err("Questa versione non può modificare il profilo.")
    }
}

#[cfg_attr(not(feature = "live"), allow(dead_code))]
fn preview_image((w, h, mut rgba): (u32, u32, Vec<u8>)) -> Option<Arc<gpui::RenderImage>> {
    // GPUI RenderImage expects BGRA.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(w, h, rgba)?;
    Some(Arc::new(gpui::RenderImage::new([image::Frame::new(buffer)])))
}

fn error_text(error: OperationError, blank: bool) -> &'static str {
    match error {
        OperationError::InvalidInput if blank => "Nome mancante",
        OperationError::InvalidInput => "Nome o immagine non validi",
        OperationError::Forbidden => "Non consentito dal server",
        _ => "Connessione non riuscita",
    }
}

impl ProfileEdit {
    fn has_picture(&self) -> bool {
        match self.avatar {
            AvatarChange::Keep => self.current.is_some(),
            AvatarChange::Remove => false,
            AvatarChange::Set(_) => true,
        }
    }

    /// The remove button exists only when there is a picture to remove.
    fn ring(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut ring = vec![self.field.read(cx).focus.clone(), self.avatar_focus.clone()];
        if self.has_picture() {
            ring.push(self.remove_focus.clone());
        }
        ring.extend([self.cancel_focus.clone(), self.submit_focus.clone()]);
        ring
    }

    fn error_line(&self, cx: &gpui::App) -> Option<&'static str> {
        self.file_error.or_else(|| self.error.map(|e| error_text(e, self.blank(cx))))
    }

    fn name(&self, cx: &gpui::App) -> String {
        self.field.read(cx).input.display_text().trim().to_owned()
    }

    fn blank(&self, cx: &gpui::App) -> bool {
        self.name(cx).is_empty()
    }

    /// None = nothing differs from what the account has.
    fn draft(&self, cx: &gpui::App) -> Option<ProfileDraft> {
        let name = Some(self.name(cx)).filter(|n| *n != self.original);
        (name.is_some() || self.avatar != AvatarChange::Keep)
            .then(|| ProfileDraft { name, avatar: self.avatar.clone() })
    }

    fn set_controls(&self, enabled: bool, cx: &mut gpui::App) {
        self.field.update(cx, |f, _| f.enabled = enabled);
    }
}

impl Shell {
    /// `allowed` comes from the homeserver capabilities (m.set_displayname / m.set_avatar_url).
    pub(crate) fn open_profile_edit(&mut self, allowed: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.drawer_accounts().into_iter().find(|a| a.current) else { return };
        let current = account.picture(|mxc| self.avatars.get(Some(mxc)));
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = allowed;
            field.placeholder = "Nome";
            field.input.replace(None, &account.name);
            field
        });
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        field.read(cx).focus.focus(window);
        self.profile_edit = Some(ProfileEdit {
            field,
            original: account.name,
            initials: account.initials,
            current,
            avatar: AvatarChange::Keep,
            preview: None,
            file_error: None,
            allowed,
            avatar_focus: cx.focus_handle(),
            remove_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            busy: false,
            error: (!allowed).then_some(OperationError::Forbidden),
        });
        cx.notify();
    }

    fn close_profile_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.profile_edit = None;
        self.list_focus.focus(window);
        cx.notify();
    }

    fn pick_profile_avatar(&mut self, cx: &mut Context<Self>) {
        if !self.profile_edit.as_ref().is_some_and(|e| e.allowed && !e.busy) {
            return;
        }
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                let file = path.clone();
                let loaded = cx.background_spawn(async move { load_preview(&file) }).await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(edit) = &mut this.profile_edit {
                        match loaded {
                            Ok(preview) => {
                                (edit.avatar, edit.preview, edit.file_error) = (AvatarChange::Set(path), Some(preview), None);
                            }
                            Err(message) => edit.file_error = Some(message),
                        }
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn remove_profile_avatar(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = self.profile_edit.as_mut().filter(|e| e.allowed && !e.busy && e.has_picture()) {
            (edit.avatar, edit.preview, edit.file_error) = (AvatarChange::Remove, None, None);
            cx.notify();
        }
    }

    pub(crate) fn submit_profile_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.profile_edit else { return };
        if edit.busy || !edit.allowed {
            return;
        }
        if edit.blank(cx) {
            edit.error = Some(OperationError::InvalidInput);
            cx.notify();
            return;
        }
        let Some(draft) = edit.draft(cx) else {
            return self.close_profile_edit(window, cx);
        };
        edit.busy = true;
        (edit.error, edit.file_error) = (None, None);
        edit.set_controls(false, cx);
        if !self.verification.save_profile(draft.name, draft.avatar) {
            // Another command is still queued: say so, the user presses Salva again.
            edit.busy = false;
            edit.error = Some(OperationError::Unavailable);
            edit.set_controls(true, cx);
        }
        cx.notify();
    }

    /// The engine's answer: success closes the dialog; an error keeps it open and Salva active again.
    pub(crate) fn profile_saved(&mut self, result: Result<(), OperationError>, cx: &mut Context<Self>) {
        let Some(edit) = self.profile_edit.as_mut().filter(|e| e.busy) else { return };
        edit.busy = false;
        match result {
            Ok(()) => {
                self.profile_edit = None;
                self.refocus_list = true;
            }
            Err(error) => {
                edit.error = Some(error);
                edit.set_controls(true, cx);
            }
        }
        cx.notify();
    }

    fn profile_edit_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.profile_edit else { return };
        let key = event.keystroke.key.as_str();
        let on = |h: &FocusHandle| h.is_focused(window);
        if key == "tab" {
            let ring = edit.ring(cx);
            let at = ring.iter().position(&on).unwrap_or(0);
            let next = if event.keystroke.modifiers.shift { (at + ring.len() - 1) % ring.len() } else { (at + 1) % ring.len() };
            ring[next].focus(window);
        } else if key == "escape" || (matches!(key, "enter" | "space") && on(&edit.cancel_focus)) {
            self.close_profile_edit(window, cx);
        } else if matches!(key, "enter" | "space") && on(&edit.avatar_focus) {
            self.pick_profile_avatar(cx);
        } else if matches!(key, "enter" | "space") && on(&edit.remove_focus) {
            self.remove_profile_avatar(cx);
        } else if key == "enter" || (key == "space" && on(&edit.submit_focus)) {
            self.submit_profile_edit(window, cx);
        } else {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_profile_edit(&self, window: &Window, cx: &gpui::App) -> Option<matrix_desktop::accessibility::new_room::Snapshot> {
        use accesskit::Role;
        use matrix_desktop::accessibility::new_room::{Item, Snapshot};
        let edit = self.profile_edit.as_ref()?;
        let item = |role, label: &str, focus: &FocusHandle, bounds| Item { role, label: label.to_owned(), checked: None, focused: focus.is_focused(window), bounds };
        let field = edit.field.read(cx);
        let mut items = vec![
            item(Role::TextInput, "Nome", &field.focus, field.semantic_bounds),
            item(Role::Button, "Avatar", &edit.avatar_focus, None),
        ];
        if edit.has_picture() {
            items.push(item(Role::Button, "Rimuovi avatar", &edit.remove_focus, None));
        }
        items.push(item(Role::Button, "Annulla", &edit.cancel_focus, None));
        items.push(item(Role::Button, "Salva", &edit.submit_focus, None));
        if let Some(error) = edit.error_line(cx) {
            items.push(Item { role: Role::Label, label: error.to_owned(), checked: None, focused: false, bounds: None });
        }
        Some(Snapshot { items, join: None })
    }

    pub(crate) fn render_profile_edit(
        &self,
        t: matrix_desktop::DesignTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let edit = self.profile_edit.as_ref()?;
        edit.field.update(cx, |f, _| f.theme = t);
        let usable = edit.allowed && !edit.busy;
        let button = |id: &'static str, label: &'static str, color: u32, focus: &FocusHandle| {
            div()
                .id(id)
                .track_focus(focus)
                .px_4()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .border_1()
                .border_color(rgb(if input_modality::only_keys(focus.is_focused(window), cx) { t.accent } else { t.surface }))
                .text_color(rgb(color))
                .child(label)
        };
        let tile = match &edit.avatar {
            AvatarChange::Set(_) if edit.preview.is_some() => crate::avatar_view::round(edit.preview.clone()?, 72.),
            AvatarChange::Keep if edit.current.is_some() => crate::avatar_view::round(edit.current.clone()?, 72.),
            _ => div()
                .size(px(72.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(matrix_desktop::AVATAR_FALLBACK))
                .text_color(rgb(t.avatar_text))
                .text_size(px(24.))
                .child(edit.initials.clone())
                .into_any_element(),
        };
        Some(
            div()
                .id("profile-edit-layer")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_profile_edit(window, cx)))
                .child(
                    div()
                        .id("profile-edit-dialog")
                        .w(px(320.))
                        .p_4()
                        .gap_2()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.profile_edit_key(event, window, cx)))
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(t.text)).child("Modifica profilo"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .id("profile-edit-avatar")
                                        .track_focus(&edit.avatar_focus)
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .border_2()
                                        .border_color(rgb(if input_modality::only_keys(edit.avatar_focus.is_focused(window), cx) { t.accent } else { t.surface }))
                                        .when(usable, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.pick_profile_avatar(cx))))
                                        .child(tile),
                                )
                                .when(edit.has_picture(), |d| {
                                    d.child(
                                        button("profile-edit-remove", "Rimuovi", if usable { t.accent } else { t.disabled }, &edit.remove_focus)
                                            .on_click(cx.listener(|this, _, _, cx| this.remove_profile_avatar(cx))),
                                    )
                                }),
                        )
                        .child(edit.field.clone())
                        .children(edit.error_line(cx).map(|error| {
                            div().text_size(px(13.)).text_color(rgb(t.error)).child(error)
                        }))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    button("profile-edit-cancel", "Annulla", t.accent, &edit.cancel_focus)
                                        .on_click(cx.listener(|this, _, window, cx| this.close_profile_edit(window, cx))),
                                )
                                .child(
                                    button("profile-edit-submit", "Salva", if usable { t.accent } else { t.disabled }, &edit.submit_focus)
                                        .on_click(cx.listener(|this, _, window, cx| this.submit_profile_edit(window, cx))),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_per_failure() {
        // A non-square picture becomes the square preview as is (the crop is the engine's job).
        assert!(preview_image((2, 2, vec![0; 16])).is_some());
        assert!(preview_image((2, 2, vec![0; 15])).is_none());
        assert_eq!(error_text(OperationError::InvalidInput, true), "Nome mancante");
        assert_eq!(error_text(OperationError::InvalidInput, false), "Nome o immagine non validi");
        assert_eq!(error_text(OperationError::Forbidden, false), "Non consentito dal server");
        assert_eq!(error_text(OperationError::Transport, false), "Connessione non riuscita");
    }
}
