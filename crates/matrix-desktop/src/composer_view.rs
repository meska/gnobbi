//! Original multiline editor over the GPUI text-input protocol.
use gpui::{
    Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, FocusHandle,
    HighlightStyle, KeyDownEvent, MouseButton, Pixels, Point, ScrollHandle, StyledText, TextLayout,
    UTF16Selection, UnderlineStyle, Window, canvas, div, fill, point, prelude::*, px, rgb, size,
};
use matrix_desktop::{Composer, DesignTokens, LIGHT, RoomDrafts};
use matrix_domain::RoomScope;
use std::ops::Range;
use std::{cell::Cell, rc::Rc};

pub(crate) struct SendVisit {
    generation: Cell<u64>,
    visible: Cell<bool>,
}
impl SendVisit {
    pub(crate) fn set_visible(&self, visible: bool) {
        if self.visible.replace(visible) != visible {
            self.retire();
        }
    }
    fn retire(&self) {
        self.generation.set(
            self.generation
                .get()
                .checked_add(1)
                .expect("Send generation exhausted"),
        );
    }
    pub(crate) fn source(&self) -> Option<u64> {
        self.visible.get().then_some(self.generation.get())
    }
}

/// File or image waiting to be sent with the draft text as caption.
struct Pending {
    filename: String,
    data: Vec<u8>,
    voice: Option<matrix_engine::sending::Voice>,
}
/// Bound on a file read into memory for the SDK upload, whatever the server allows.
const MAX_ATTACHMENT: u64 = 50 * 1024 * 1024;
/// Upload limit assumed until the server's is known: Tuwunel's default, the smallest in use.
const DEFAULT_UPLOAD_LIMIT: u64 = 24 * 1024 * 1024;

pub struct Submitted(pub String);

impl gpui::EventEmitter<Submitted> for ComposerView {}

pub struct ComposerView {
    pub(crate) visit: Rc<SendVisit>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    semantic_bounds: Option<accesskit::Rect>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    send_bounds: Option<(u64, accesskit::Rect)>,
    live: bool,
    draft: Composer,
    attachment: Option<Pending>,
    /// Joined members of the shown room, offered by the @ completion.
    members: Vec<matrix_engine::operations::Member>,
    /// Users picked from the completion for the draft being written.
    mentions: Vec<matrix_engine::sending::Mention>,
    mention_selected: usize,
    /// Start of the `@` whose list was closed with Esc.
    mention_closed: Option<usize>,
    emoji_open: bool,
    emoji_selected: usize,
    attach_error: Option<&'static str>,
    /// The homeserver's upload limit (media config), once read.
    upload_limit: Option<u64>,
    recorder: Option<crate::voice_recorder::Recorder>,
    rooms: RoomDrafts,
    focus: FocusHandle,
    send_focus: FocusHandle,
    send_focus_source: u64,
    layout: Option<TextLayout>,
    scroll: ScrollHandle,
    anchor: usize,
    cursor: usize,
    dragging: bool,
    reveal: bool,
    theme: DesignTokens,
}

impl ComposerView {
    pub fn new(scope: RoomScope, cx: &mut Context<Self>) -> Self {
        Self {
            visit: Rc::new(SendVisit {
                generation: Cell::new(0),
                visible: Cell::new(true),
            }),
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            semantic_bounds: None,
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            send_bounds: None,
            live: false,
            draft: Composer::default(),
            attachment: None,
            members: Vec::new(),
            mentions: Vec::new(),
            mention_selected: 0,
            mention_closed: None,
            emoji_open: false,
            emoji_selected: 0,
            attach_error: None,
            upload_limit: None,
            recorder: None,
            rooms: RoomDrafts::new(scope),
            focus: cx.focus_handle(),
            send_focus: cx.focus_handle(),
            send_focus_source: 0,
            layout: None,
            scroll: ScrollHandle::new(),
            anchor: 0,
            cursor: 0,
            dragging: false,
            reveal: false,
            theme: LIGHT,
        }
    }

    pub fn switch_room(&mut self, scope: RoomScope, cx: &mut Context<Self>) {
        self.rooms.switch(scope, &mut self.draft);
        // A pending file belongs to the room it was added in, never to the next one.
        self.attachment = None;
        self.members.clear();
        self.mentions.clear();
        self.emoji_open = false;
        self.attach_error = None;
        self.recorder = None;
        self.layout = None;
        self.scroll = ScrollHandle::new();
        self.dragging = false;
        self.changed(cx);
    }

    pub fn set_upload_limit(&mut self, limit: Option<u64>) {
        self.upload_limit = limit;
    }

    /// Largest file that can be attached: the server limit, within the in-memory bound.
    fn attachment_cap(&self) -> u64 {
        self.upload_limit.map_or(MAX_ATTACHMENT, |l| l.min(MAX_ATTACHMENT))
    }

    pub fn set_members(
        &mut self,
        members: Vec<matrix_engine::operations::Member>,
        cx: &mut Context<Self>,
    ) {
        self.members = members;
        cx.notify();
    }

    /// Members offered for the `@` before the caret; empty when no completion is open.
    fn mention_options(&self) -> Vec<matrix_engine::operations::Member> {
        if !self.live || self.draft.marked().is_some() || !self.draft.selection().is_empty() {
            return Vec::new();
        }
        match matrix_desktop::mention_token(self.draft.text(), self.cursor) {
            Some((at, query)) if self.mention_closed != Some(at) => {
                matrix_desktop::mention_matches(&self.members, query)
                    .into_iter()
                    .cloned()
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn accept_mention(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(member) = self.mention_options().into_iter().nth(index) else {
            return;
        };
        let Some((at, _)) = matrix_desktop::mention_token(self.draft.text(), self.cursor) else {
            return;
        };
        let label = if member.name.starts_with('@') {
            member.name.clone()
        } else {
            format!("@{}", member.name)
        };
        self.draft.select(at..self.cursor);
        if self.draft.replace(None, &format!("{label} ")) {
            self.mentions.push(matrix_engine::sending::Mention {
                user_id: member.user_id,
                label,
            });
        }
        self.changed(cx);
    }

    /// Insert `EMOJI[index]` at the caret (over the selection) and give the focus back to the text.
    fn pick_emoji(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.emoji_open = false;
        if let Some((_, emoji)) = matrix_desktop::EMOJI_SET.get(index)
            && self.draft.marked().is_none()
        {
            self.draft.replace(None, emoji);
        }
        window.focus(&self.focus);
        self.changed(cx);
    }

    pub fn set_live(&mut self, live: bool) {
        self.live = live;
    }

    #[cfg(feature = "live")]
    pub fn accept_live(
        &mut self,
        delivery: &mut matrix_desktop::LiveDelivery,
        scope: RoomScope,
        transaction_id: matrix_domain::TransactionId,
        cx: &mut Context<Self>,
    ) {
        let mut file = self
            .attachment
            .take()
            .map(|p| matrix_engine::sending::Attachment {
                filename: p.filename,
                data: p.data,
                voice: p.voice,
            });
        if delivery.submit_mentioning(
            scope,
            transaction_id,
            &mut self.draft,
            &mut file,
            self.mentions.clone(),
        ) {
            self.mentions.clear();
        }
        // Not accepted (queue full): keep the file for the next try.
        self.attachment = file.map(|f| Pending {
            filename: f.filename,
            data: f.data,
            voice: f.voice,
        });
        self.changed(cx);
    }

    #[cfg(feature = "live")]
    pub fn accept_operation(
        &mut self,
        operations: &mut matrix_desktop::LiveOperations,
        edit: bool,
        transaction_id: matrix_domain::TransactionId,
        cx: &mut Context<Self>,
    ) {
        operations.submit_draft(edit, transaction_id, &mut self.draft);
        self.changed(cx);
    }

    pub fn set_theme(&mut self, theme: DesignTokens, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn draft_state(&self) -> (String, usize, usize) {
        (self.draft.text().to_owned(), self.anchor, self.cursor)
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window) || self.send_is_focused(window)
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn semantic_snapshot(
        &self,
        scope: RoomScope,
        window: &Window,
    ) -> matrix_desktop::accessibility::ComposerSnapshot {
        matrix_desktop::accessibility::ComposerSnapshot {
            send_source: self.visit.source(),
            scope,
            composing: self.draft.marked().is_some(),
            send_focused: self.send_is_focused(window),
            attachment: self.attachment.as_ref().map(|p| p.filename.clone()),
            mentions: Some(self.mention_options())
                .filter(|o| !o.is_empty())
                .map(|o| {
                    (
                        o.into_iter().map(|m| m.name).collect(),
                        self.mention_selected,
                    )
                }),
            recording: self.recorder.as_ref().map(|r| r.elapsed_secs()),
            emoji_open: self.emoji_open,
            send_bounds: self
                .send_bounds
                .filter(|(source, _)| self.visit.source() == Some(*source))
                .map(|(_, rect)| rect),
            text: self.draft.text().to_owned(),
            anchor: self.anchor,
            cursor: self.cursor,
            focused: self.focus.is_focused(window),
            bounds: self.semantic_bounds,
        }
    }

    /// The text field itself (not Send) holds focus and no IME preedit is active.
    pub fn tab_back_ready(&self, window: &Window) -> bool {
        self.focus.is_focused(window) && self.draft.marked().is_none()
    }

    /// Tab from the text field reaches Send only for a non-blank draft outside IME preedit.
    pub fn send_eligible(&self) -> bool {
        self.draft.marked().is_none() && !self.draft.text().trim().is_empty()
    }

    pub fn focus_from_accessibility(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        cx.notify();
    }

    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn submit_from_accessibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        self.submit(cx);
    }

    pub fn focus_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_send_focus(cx);
        self.send_focus.focus(window);
        cx.notify();
    }

    pub fn send_is_focused(&self, window: &Window) -> bool {
        self.visit.source() == Some(self.send_focus_source) && self.send_focus.is_focused(window)
    }

    fn refresh_send_focus(&mut self, cx: &mut Context<Self>) {
        if self.send_focus_source != self.visit.generation.get() {
            self.send_focus = cx.focus_handle();
            self.send_focus_source = self.visit.generation.get();
        }
    }

    fn send_observer(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
        {
            let entity = cx.entity();
            let source = self.visit.source();
            Some(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let scale = f64::from(window.scale_factor());
                        let rect = accesskit::Rect::new(
                            f64::from(f32::from(bounds.left())) * scale,
                            f64::from(f32::from(bounds.top())) * scale,
                            f64::from(f32::from(bounds.right())) * scale,
                            f64::from(f32::from(bounds.bottom())) * scale,
                        );
                        if let Some(source) = source
                            && entity.read(cx).send_bounds != Some((source, rect))
                        {
                            let entity = entity.clone();
                            window.on_next_frame(move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    if this.visit.source() == Some(source) {
                                        this.send_bounds = Some((source, rect));
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
            let _ = cx;
            None
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.visit.retire();
        self.reveal = true;
        self.mention_selected = 0;
        self.cursor = self.draft.selection().end;
        self.anchor = self.draft.selection().start;
        if matrix_desktop::mention_token(self.draft.text(), self.cursor).map(|(at, _)| at)
            != self.mention_closed
        {
            self.mention_closed = None;
        }
        cx.notify();
    }

    fn choose(&mut self, index: usize, extend: bool, cx: &mut Context<Self>) {
        let index = index.min(self.draft.text().len());
        if !self.draft.text().is_char_boundary(index) {
            return;
        }
        if !extend {
            self.anchor = index;
        }
        self.cursor = index;
        self.reveal = true;
        self.draft
            .select(self.anchor.min(index)..self.anchor.max(index));
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.live {
            if self.sendable() {
                cx.emit(Submitted(self.draft.text().to_owned()));
            }
        } else if let Some(text) = self.draft.enter(false) {
            cx.emit(Submitted(text));
        }
        self.changed(cx);
    }

    fn sendable(&self) -> bool {
        self.draft.marked().is_none()
            && (!self.draft.text().trim().is_empty() || self.attachment.is_some())
    }

    /// Reads the first path off the UI thread and keeps it as the pending attachment.
    pub fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = paths.into_iter().next().filter(|_| self.live) else {
            return;
        };
        let cap = self.attachment_cap();
        let read = cx.background_executor().spawn(async move {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let size = std::fs::metadata(&path).ok()?.len();
            Some((
                name,
                (0 < size && size <= cap)
                    .then(|| std::fs::read(&path).ok())
                    .flatten(),
            ))
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = this.update(cx, |this, cx| {
                match read {
                    Some((filename, Some(data))) => {
                        this.attachment = Some(Pending {
                            filename,
                            data,
                            voice: None,
                        });
                        this.attach_error = None;
                    }
                    _ => this.attach_error = Some("File non allegabile"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn attach_image(&mut self, image: &gpui::Image, cx: &mut Context<Self>) {
        // Synapse rejects zero-byte uploads, so an empty paste never becomes attachable.
        if image.bytes.is_empty() || image.bytes.len() as u64 > self.attachment_cap() {
            self.attach_error = Some("File non allegabile");
            cx.notify();
            return;
        }
        let ext = image.format.mime_type().rsplit('/').next().unwrap_or("png");
        let ext = ext.split('+').next().unwrap_or(ext);
        self.attachment = Some(Pending {
            filename: format!("immagine.{ext}"),
            data: image.bytes.clone(),
            voice: None,
        });
        self.attach_error = None;
        cx.notify();
    }

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        match crate::voice_recorder::Recorder::start(self.upload_limit.unwrap_or(DEFAULT_UPLOAD_LIMIT)) {
            Ok(recorder) => {
                self.recorder = Some(recorder);
                self.attach_error = None;
                // Repaints the elapsed time until the recording ends.
                cx.spawn(async move |this, cx| {
                    while this
                        .update(cx, |this, cx| {
                            // At the cap the note goes out like «Invia nota».
                            if this.recorder.as_ref().is_some_and(|r| r.is_full()) {
                                this.stop_recording(true, cx);
                            }
                            cx.notify();
                            this.recorder.is_some()
                        })
                        .unwrap_or(false)
                    {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(500))
                            .await;
                    }
                })
                .detach();
            }
            Err(error) => self.attach_error = Some(error),
        }
        cx.notify();
    }

    /// Ends the recording; `send` queues the note, otherwise it is discarded.
    fn stop_recording(&mut self, send: bool, cx: &mut Context<Self>) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        let (samples, rate) = recorder.finish();
        if send {
            match matrix_desktop::voice_attachment(&samples, rate, self.upload_limit.unwrap_or(DEFAULT_UPLOAD_LIMIT)) {
                Some(note) => {
                    self.attachment = Some(Pending {
                        filename: note.filename,
                        data: note.data,
                        voice: note.voice,
                    });
                    self.submit(cx);
                }
                None => self.attach_error = Some("Nessun audio registrato"),
            }
        }
        cx.notify();
    }

    fn pick_file(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| this.attach_paths(paths, cx));
            }
        })
        .detach();
    }

    pub(crate) fn send_callback(
        &self,
    ) -> impl Fn(&mut Self, &mut Window, &mut Context<Self>) + use<> {
        let source = self.visit.source();
        move |this, window, cx| {
            if source.is_some() && this.visit.source() == source && this.sendable() {
                this.focus.focus(window);
                this.submit(cx);
            }
        }
    }

    pub(crate) fn key_callback(
        &self,
    ) -> impl Fn(&mut Self, &KeyDownEvent, &mut Window, &mut Context<Self>) + use<> {
        self.key_callback_for_target(cfg!(target_os = "macos"))
    }

    fn key_callback_for_target(
        &self,
        mac: bool,
    ) -> impl Fn(&mut Self, &KeyDownEvent, &mut Window, &mut Context<Self>) + use<> {
        let source = self.visit.source();
        move |this, event, window, cx| {
            if source.is_some() && this.visit.source() == source {
                this.key_down_for_target(event, mac, window, cx);
            }
        }
    }

    #[cfg(test)]
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.key_down_for_target(event, cfg!(target_os = "macos"), window, cx);
    }

    fn key_down_for_target(
        &mut self,
        event: &KeyDownEvent,
        mac: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_send_focus(cx);
        let key = &event.keystroke;
        if self.emoji_open && key.key == "escape" && self.focus.is_focused(window) {
            self.emoji_open = false;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.emoji_open && self.focus.is_focused(window) {
            let last = matrix_desktop::EMOJI_SET.len() - 1;
            match key.key.as_str() {
                "right" | "down" => {
                    self.emoji_selected = if self.emoji_selected >= last {
                        0
                    } else {
                        self.emoji_selected + 1
                    }
                }
                "left" | "up" => {
                    self.emoji_selected = self.emoji_selected.checked_sub(1).unwrap_or(last)
                }
                "enter" => self.pick_emoji(self.emoji_selected, window, cx),
                _ => return self.key_down_text(event, mac, window, cx),
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let options = self.mention_options();
        if !options.is_empty() && self.focus.is_focused(window) {
            let last = options.len() - 1;
            let current = self.mention_selected.min(last);
            match key.key.as_str() {
                "down" => self.mention_selected = if current == last { 0 } else { current + 1 },
                "up" => self.mention_selected = if current == 0 { last } else { current - 1 },
                "enter" | "tab" => self.accept_mention(current, cx),
                "escape" => {
                    self.mention_closed =
                        matrix_desktop::mention_token(self.draft.text(), self.cursor)
                            .map(|(at, _)| at);
                }
                _ => {
                    self.mention_selected = current;
                    return self.key_down_text(event, mac, window, cx);
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        self.key_down_text(event, mac, window, cx)
    }

    fn key_down_text(
        &mut self,
        event: &KeyDownEvent,
        mac: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        if key.key == "tab"
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && self.draft.marked().is_none()
        {
            if self.send_focus.is_focused(window) && key.modifiers.shift {
                self.focus.focus(window);
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self.focus.is_focused(window)
                && !key.modifiers.shift
                && !self.draft.text().trim().is_empty()
            {
                self.send_focus.focus(window);
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        if self.send_focus.is_focused(window) {
            return;
        }
        // Preedit belongs to the input method, not to chat shortcuts.
        if self.draft.marked().is_some() {
            return;
        }
        let selected = self.draft.selection();
        if key.modifiers.control
            || (crate::editing_command::command_modifier(&key.modifiers, mac)
                && matches!(key.key.as_str(), "a" | "c" | "x" | "v" | "z" | "y"))
        {
            match key.key.as_str() {
                "a" => {
                    self.anchor = 0;
                    self.choose(self.draft.text().len(), true, cx);
                }
                "c" | "x" => {
                    if !selected.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.draft.text()[selected].to_owned(),
                        ));
                        if key.key == "x" {
                            self.draft.replace(None, "");
                            self.changed(cx);
                        }
                    }
                }
                "v" => {
                    let item = cx.read_from_clipboard();
                    let image = self.live.then(|| {
                        item.as_ref()?
                            .entries()
                            .iter()
                            .find_map(|entry| match entry {
                                gpui::ClipboardEntry::Image(image) => Some(image.clone()),
                                _ => None,
                            })
                    });
                    if let Some(Some(image)) = image {
                        self.attach_image(&image, cx);
                    } else if let Some(text) = item.and_then(|item| item.text()) {
                        self.draft.replace(None, &text);
                        self.changed(cx);
                    }
                }
                "z" if key.modifiers.shift => {
                    self.draft.redo();
                    self.changed(cx);
                }
                "z" => {
                    self.draft.undo();
                    self.changed(cx);
                }
                "y" => {
                    self.draft.redo();
                    self.changed(cx);
                }
                "home" => self.choose(0, key.modifiers.shift, cx),
                "end" => self.choose(self.draft.text().len(), key.modifiers.shift, cx),
                _ => return,
            }
        } else {
            match key.key.as_str() {
                "enter" if key.modifiers.shift => {
                    self.draft.enter(true);
                    self.changed(cx);
                }
                "enter" => self.submit(cx),
                "left" | "right" | "backspace" | "delete" => {
                    let previous = self.draft.text()[..self.cursor]
                        .char_indices()
                        .last()
                        .map_or(0, |(i, _)| i);
                    let next = self.draft.text()[self.cursor..]
                        .chars()
                        .next()
                        .map_or(self.cursor, |c| self.cursor + c.len_utf8());
                    match key.key.as_str() {
                        "left" => self.choose(
                            if !selected.is_empty() && !key.modifiers.shift {
                                selected.start
                            } else {
                                previous
                            },
                            key.modifiers.shift,
                            cx,
                        ),
                        "right" => self.choose(
                            if !selected.is_empty() && !key.modifiers.shift {
                                selected.end
                            } else {
                                next
                            },
                            key.modifiers.shift,
                            cx,
                        ),
                        _ => {
                            if selected.is_empty() {
                                self.draft.select(if key.key == "backspace" {
                                    previous..self.cursor
                                } else {
                                    self.cursor..next
                                });
                            }
                            self.draft.replace(None, "");
                            self.changed(cx);
                        }
                    }
                }
                "home" => {
                    let start = self.draft.text()[..self.cursor]
                        .rfind('\n')
                        .map_or(0, |i| i + 1);
                    self.choose(start, key.modifiers.shift, cx);
                }
                "end" => {
                    let end = self.draft.text()[self.cursor..]
                        .find('\n')
                        .map_or(self.draft.text().len(), |i| self.cursor + i);
                    self.choose(end, key.modifiers.shift, cx);
                }
                _ => return,
            }
        }
        cx.stop_propagation();
    }
}

impl EntityInputHandler for ComposerView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.draft.from_utf16(range);
        *actual = Some(self.draft.to_utf16(range.clone()));
        self.draft.text().get(range).map(str::to_owned)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.draft.to_utf16(self.draft.selection()),
            reversed: self.cursor < self.anchor,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.draft.marked().map(|range| self.draft.to_utf16(range))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.draft.unmark();
        self.changed(cx);
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|r| self.draft.from_utf16(r));
        self.draft.replace(range, text);
        self.changed(cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|r| self.draft.from_utf16(r));
        self.draft.mark(range, text, selected);
        self.changed(cx);
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.draft.from_utf16(range);
        let layout = self.layout.as_ref()?;
        let p = layout.position_for_index(range.start)?;
        Some(Bounds::new(p, size(px(2.), px(20.))))
    }
    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let i = self
            .layout
            .as_ref()?
            .index_for_position(p)
            .unwrap_or_else(|i| i)
            .min(self.draft.text().len());
        Some(self.draft.to_utf16(i..i).start)
    }
}

impl Render for ComposerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_send_focus(cx);
        let t = self.theme;
        let empty = self.draft.text().is_empty();
        // The layout-only sentinel provides a caret position after a final newline.
        // It is never part of the draft, clipboard, input protocol or submitted body.
        let text = if empty {
            "Scrivi un messaggio".to_owned()
        } else {
            format!("{}\u{200b}", self.draft.text())
        };
        let mut highlights = Vec::new();
        let selected = self.draft.selection();
        if !selected.is_empty() {
            highlights.push((
                selected,
                HighlightStyle {
                    background_color: Some(rgb(t.selection).into()),
                    ..Default::default()
                },
            ));
        }
        if let Some(marked) = self.draft.marked() {
            highlights.push((
                marked,
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        color: Some(rgb(t.accent).into()),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ));
        }
        let sendable = self.sendable();
        let options = self.mention_options();
        let mention_selected = self.mention_selected.min(options.len().saturating_sub(1));
        let attachment_name = self.attachment.as_ref().map(|p| p.filename.clone());
        let body = StyledText::new(text).with_highlights(highlights);
        let layout = body.layout().clone();
        let entity = cx.entity();
        div()
            .id("composer")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(self.key_callback()))
            .when(cfg!(any(target_os = "macos", test)), |element| {
                let key = self.key_callback_for_target(true);
                crate::editing_command::with_actions(
                    element,
                    true,
                    cx.listener(move |this, event, window, cx| {
                        if this.focus.is_focused(window) {
                            key(this, event, window, cx);
                        }
                    }),
                )
            })
            .border_t_1()
            .border_color(rgb(t.border))
            .min_h(px(t.composer))
            .flex_shrink_0()
            .px_6()
            .py_2()
            .flex()
            .flex_col()
            .justify_center()
            .gap_1()
            .when(!options.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .children(options.into_iter().enumerate().map(|(i, member)| {
                            div()
                                .id(("composer-mention", i))
                                .cursor_pointer()
                                .px_2()
                                .py_1()
                                .flex()
                                .gap_2()
                                .min_w_0()
                                .overflow_hidden()
                                .text_size(px(14.))
                                .text_color(rgb(t.text))
                                .when(i == mention_selected, |d| d.bg(rgb(t.selection)))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .child(matrix_desktop::row_text::one_line(&member.name)),
                                )
                                .when(member.name != member.user_id, |d| {
                                    d.child(
                                        div()
                                            .flex_shrink_0()
                                            .max_w(px(160.))
                                            .truncate()
                                            .text_color(rgb(t.secondary))
                                            .child(matrix_desktop::row_text::one_line(
                                                &member.user_id,
                                            )),
                                    )
                                })
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.accept_mention(i, cx)),
                                )
                        })),
                )
            })
            .when(self.emoji_open, |d| {
                d.child(
                    div().flex().flex_wrap().gap_1().children(
                        matrix_desktop::EMOJI_SET
                            .iter()
                            .enumerate()
                            .map(|(i, (_, emoji))| {
                                div()
                                    .when(i == self.emoji_selected, |d| d.bg(rgb(t.selection)))
                                    .id(("composer-emoji", i))
                                    .cursor_pointer()
                                    .px_1()
                                    .text_size(px(20.))
                                    .child(*emoji)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.pick_emoji(i, window, cx)
                                    }))
                            }),
                    ),
                )
            })
            .when(
                attachment_name.is_some() || self.attach_error.is_some(),
                |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .text_color(rgb(if self.attach_error.is_some() {
                                t.text
                            } else {
                                t.secondary
                            }))
                            .child(attachment_name.clone().unwrap_or_default())
                            .children(self.attach_error)
                            .child(
                                div()
                                    .id("composer-attachment-remove")
                                    .cursor_pointer()
                                    .px_2()
                                    .child("×")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.attachment = None;
                                        this.attach_error = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .when(self.live && self.recorder.is_none(), |d| {
                        d.child(
                            div()
                                .id("composer-attach")
                                .cursor_pointer()
                                .flex_shrink_0()
                                .px_2()
                                .py_2()
                                .text_color(rgb(t.accent))
                                .child("Allega")
                                .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
                        )
                        .child(
                            div()
                                .id("composer-record")
                                .cursor_pointer()
                                .flex_shrink_0()
                                .px_2()
                                .py_2()
                                .text_color(rgb(t.accent))
                                .child("Voce")
                                .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                        )
                        .child(
                            div()
                                .id("composer-emoji-toggle")
                                .cursor_pointer()
                                .flex_shrink_0()
                                .px_2()
                                .py_2()
                                .text_color(rgb(t.accent))
                                .child("Emoji")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.emoji_open = !this.emoji_open;
                                    this.emoji_selected = 0;
                                    cx.notify();
                                })),
                        )
                    })
                    .when_some(
                        self.recorder.as_ref().map(|r| r.elapsed_secs()),
                        |d, secs| {
                            d.child(
                                div()
                                    .id("composer-recording")
                                    .flex_shrink_0()
                                    .px_2()
                                    .text_color(rgb(t.error))
                                    .child(format!("● {}:{:02}", secs / 60, secs % 60)),
                            )
                            .child(
                                div()
                                    .id("composer-record-cancel")
                                    .cursor_pointer()
                                    .flex_shrink_0()
                                    .px_2()
                                    .py_2()
                                    .text_color(rgb(t.secondary))
                                    .child("Annulla")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.stop_recording(false, cx)
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("composer-record-send")
                                    .cursor_pointer()
                                    .flex_shrink_0()
                                    .px_2()
                                    .py_2()
                                    .text_color(rgb(t.accent))
                                    .child("Invia nota")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.stop_recording(true, cx)),
                                    ),
                            )
                        },
                    )
                    .child(
                        div()
                            .id("composer-editor")
                            .when(self.recorder.is_some(), |d| d.hidden())
                            .flex_1()
                            .min_w_0()
                            .min_h(px(36.))
                            .relative()
                            .cursor(gpui::CursorStyle::IBeam)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                                    this.focus.focus(window);
                                    this.dragging = true;
                                    let i = this.layout.as_ref().map_or(0, |l| {
                                        l.index_for_position(e.position).unwrap_or_else(|i| i)
                                    });
                                    this.choose(i, e.modifiers.shift, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                                if this.dragging && e.pressed_button == Some(MouseButton::Left) {
                                    let i = this.layout.as_ref().map_or(0, |l| {
                                        l.index_for_position(e.position).unwrap_or_else(|i| i)
                                    });
                                    this.choose(i, true, cx);
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| this.dragging = false),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| this.dragging = false),
                            )
                            .max_h(px(120.))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .whitespace_normal()
                            .text_color(rgb(if empty { t.secondary } else { t.text }))
                            .child(body)
                            .child(
                                canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, cx| {
                                        #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                                        {
                                            let scale = f64::from(window.scale_factor());
                                            let rect = accesskit::Rect::new(
                                                f64::from(f32::from(bounds.left())) * scale,
                                                f64::from(f32::from(bounds.top())) * scale,
                                                f64::from(f32::from(bounds.right())) * scale,
                                                f64::from(f32::from(bounds.bottom())) * scale,
                                            );
                                            if entity.read(cx).semantic_bounds != Some(rect) {
                                                let entity = entity.clone();
                                                window.on_next_frame(move |_, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.semantic_bounds = Some(rect);
                                                        cx.notify();
                                                    });
                                                });
                                            }
                                        }
                                        let this = entity.read(cx);
                                        window.handle_input(
                                            &this.focus,
                                            ElementInputHandler::new(bounds, entity.clone()),
                                            cx,
                                        );
                                        if this.focus.is_focused(window)
                                            && this.draft.selection().is_empty()
                                            && let Some(p) = layout.position_for_index(this.cursor)
                                        {
                                            window.paint_quad(fill(
                                                Bounds::new(point(p.x, p.y), size(px(1.), px(20.))),
                                                rgb(t.accent),
                                            ));
                                        }
                                        entity.update(cx, |this, _| {
                                            if this.reveal
                                                && let Some(p) =
                                                    layout.position_for_index(this.cursor)
                                            {
                                                let viewport = this.scroll.bounds();
                                                let delta = if p.y < viewport.top() {
                                                    viewport.top() - p.y
                                                } else if p.y + px(20.) > viewport.bottom() {
                                                    viewport.bottom() - p.y - px(20.)
                                                } else {
                                                    px(0.)
                                                };
                                                if delta.abs() > px(0.5) {
                                                    let y = (this.scroll.offset().y + delta).clamp(
                                                        -this.scroll.max_offset().height,
                                                        px(0.),
                                                    );
                                                    // Geometry is final only after prepaint. Invalidate on the
                                                    // next frame: a paint-time notify is consumed by this draw.
                                                    let entity = entity.clone();
                                                    window.on_next_frame(move |_, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.scroll
                                                                .set_offset(point(px(0.), y));
                                                            cx.notify();
                                                        });
                                                    });
                                                }
                                            }
                                            this.reveal = false;
                                            this.layout = Some(layout.clone());
                                        });
                                    },
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    )
                    .child(
                        div()
                            .id("composer-submit")
                            .relative()
                            .children(self.send_observer(cx))
                            .track_focus(&self.send_focus)
                            .border_1()
                            .border_color(rgb(if self.send_focus.is_focused(window) {
                                t.text
                            } else if sendable {
                                t.accent
                            } else {
                                t.disabled
                            }))
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .flex_shrink_0()
                            .bg(rgb(if sendable { t.accent } else { t.disabled }))
                            .text_color(rgb(if sendable { t.on_accent } else { t.secondary }))
                            .when(sendable, |d| {
                                let send = self.send_callback();
                                d.cursor_pointer().on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        send(this, window, cx);
                                    },
                                ))
                            })
                            .child(if self.live { "Invia" } else { "Invia · demo" }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires private display"]
    fn edit_menu_dispatches_to_focused_composer() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    cx.new(|cx| {
                        let mut v = ComposerView::new(
                            RoomScope {
                                account_id: matrix_domain::AccountId::new(1),
                                room_id: matrix_domain::RoomId::new("!menu:fixture.invalid"),
                            },
                            cx,
                        );
                        v.replace_text_in_range(None, "caffè 🦀\nseconda riga", window, cx);
                        v.choose(0, false, cx);
                        v.choose(v.draft.text().len(), true, cx);
                        v.focus.focus(window);
                        v
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, |_, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                    window.dispatch_action(Box::new(crate::editing_command::Copy), cx);
                })
                .unwrap();
                executor.timer(std::time::Duration::from_millis(80)).await;
                view.update(cx, |v, _, cx| {
                    let copied = cx.read_from_clipboard().and_then(|i| i.text());
                    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                    assert_eq!(copied.as_deref(), Some(v.draft.text()));
                })
                .unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn edit_menu_composer_commands_reject_retired_sources() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let submissions = Rc::new(Cell::new(0));
            let observed = submissions.clone();
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    window.set_window_title("Dime private composer menu");
                    cx.new(|cx| {
                        cx.subscribe(&cx.entity(), move |_, _, _: &Submitted, _| {
                            observed.set(observed.get() + 1)
                        })
                        .detach();
                        let mut v = ComposerView::new(
                            RoomScope {
                                account_id: matrix_domain::AccountId::new(1),
                                room_id: matrix_domain::RoomId::new("!menu:fixture.invalid"),
                            },
                            cx,
                        );
                        v.set_live(true);
                        v
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(300)).await;
                for eligibility in [
                    "current",
                    "queued-blur",
                    "queued-hidden-return",
                    "queued-room",
                    "ime",
                ] {
                    for index in 0..4 {
                        view.update(cx, |v, window, cx| {
                            v.visit.set_visible(true);
                            v.draft = Composer::default();
                            v.replace_text_in_range(None, "caffè 🦀\nseconda riga", window, cx);
                            v.choose(0, false, cx);
                            if index != 3 {
                                v.choose(v.draft.text().len(), true, cx);
                            }
                            if eligibility == "ime" {
                                v.replace_and_mark_text_in_range(
                                    Some(0..v.draft.text().encode_utf16().count()),
                                    "preedit",
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            v.focus.focus(window);
                            cx.notify();
                        })
                        .unwrap();
                        executor.timer(std::time::Duration::from_millis(80)).await;
                        cx.update(|cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                            assert!(cx.active_window().is_some());
                            cx.dispatch_action(
                                crate::editing_command::tests::menu_action(index).as_ref(),
                            );
                            view.update(cx, |v, window, cx| match eligibility {
                                "queued-blur" => cx.focus_handle().focus(window),
                                "queued-hidden-return" => {
                                    v.visit.set_visible(false);
                                    v.visit.set_visible(true);
                                }
                                "queued-room" => {
                                    v.switch_room(
                                        RoomScope {
                                            account_id: matrix_domain::AccountId::new(2),
                                            room_id: matrix_domain::RoomId::new(
                                                "!other:fixture.invalid",
                                            ),
                                        },
                                        cx,
                                    );
                                    v.draft = Composer::default();
                                    v.replace_text_in_range(None, "replacement", window, cx);
                                }
                                _ => {}
                            })
                            .unwrap();
                        })
                        .unwrap();
                        executor.timer(std::time::Duration::from_millis(80)).await;
                        view.update(cx, |v, window, cx| {
                            let original = "caffè 🦀\nseconda riga";
                            let expected = match (eligibility, index) {
                                ("current", 0) => "",
                                ("current", 2) => "sentinel",
                                ("queued-room", _) => "replacement",
                                ("ime", _) => "preedit",
                                _ => original,
                            };
                            assert_eq!(v.draft.text(), expected, "{eligibility}/{index}");
                            let copied = cx.read_from_clipboard().and_then(|i| i.text());
                            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                            assert_eq!(
                                copied.as_deref(),
                                Some(if eligibility == "current" && matches!(index, 0 | 1) {
                                    original
                                } else {
                                    "sentinel"
                                })
                            );
                            if eligibility == "current" && index == 3 {
                                assert_eq!(v.draft.selection(), 0..original.len());
                            }
                            if eligibility == "queued-blur" {
                                assert!(!v.focus.is_focused(window));
                            }
                            if eligibility == "ime" {
                                assert!(v.draft.marked().is_some());
                            }
                        })
                        .unwrap();
                    }
                }
                assert_eq!(submissions.get(), 0);
                eprintln!("20 composer menu cases, zero Submitted, passed");
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    fn command_probe(
        check: impl FnOnce(&mut ComposerView, &mut Window, &mut Context<ComposerView>) + 'static,
    ) {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    cx.new(|cx| {
                        let mut v = ComposerView::new(
                            RoomScope {
                                account_id: matrix_domain::AccountId::new(1),
                                room_id: matrix_domain::RoomId::new("!commands:fixture.invalid"),
                            },
                            cx,
                        );
                        v.replace_text_in_range(None, "caffè 🦀\nseconda riga", window, cx);
                        v.focus.focus(window);
                        v
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                // Wayland reads the clipboard only through an offer from the compositor: seed one and let it arrive.
                executor.timer(std::time::Duration::from_millis(300)).await;
                cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("seed".into())))
                    .unwrap();
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, check).unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    fn command(
        v: &mut ComposerView,
        chord: &str,
        mac: bool,
        window: &mut Window,
        cx: &mut Context<ComposerView>,
    ) {
        (v.key_callback_for_target(mac))(
            v,
            &KeyDownEvent {
                keystroke: gpui::Keystroke::parse(chord).unwrap(),
                is_held: false,
            },
            window,
            cx,
        );
    }

    #[test]
    #[ignore = "requires private display"]
    fn mac_command_selects_all_composer_unicode() {
        command_probe(|v, window, cx| {
            command(v, "cmd-a", true, window, cx);
            assert_eq!(
                v.draft.selection(),
                0..v.draft.text().len(),
                "Mac Cmd+A must select the entire multiline draft"
            );
            assert_eq!((v.anchor, v.cursor), (0, v.draft.text().len()));
            let original = v.draft.text().to_owned();
            command(v, "cmd-c", true, window, cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                Some(original.as_str())
            );
            command(v, "cmd-x", true, window, cx);
            assert_eq!(v.draft.text(), "");
            assert_eq!(
                cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                Some(original.as_str())
            );
            command(v, "cmd-v", true, window, cx);
            assert_eq!(v.draft.text(), original);
            assert_eq!(v.draft.selection(), original.len()..original.len());
            command(v, "cmd-z", true, window, cx);
            assert_eq!(v.draft.text(), "");
            command(v, "shift-cmd-z", true, window, cx);
            assert_eq!(v.draft.text(), original);
            command(v, "cmd-z", true, window, cx);
            command(v, "cmd-y", true, window, cx);
            assert_eq!(v.draft.text(), original);
            cx.write_to_clipboard(ClipboardItem::new_string("".to_owned()));
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn command_routing_preserves_ctrl_sources_ime_and_enter_policy() {
        command_probe(|v, window, cx| {
            let original = v.draft.text().to_owned();
            for mac in [false, true] {
                command(v, "ctrl-a", mac, window, cx);
                command(v, "ctrl-c", mac, window, cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                    Some(original.as_str()),
                    "ctrl-c mac={mac}"
                );
                command(v, "ctrl-x", mac, window, cx);
                assert_eq!(v.draft.text(), "");
                command(v, "ctrl-v", mac, window, cx);
                assert_eq!(v.draft.text(), original);
                command(v, "ctrl-z", mac, window, cx);
                assert_eq!(v.draft.text(), "");
                command(v, "ctrl-y", mac, window, cx);
                assert_eq!(v.draft.text(), original);
            }
            // The painted entry always supplies the compile target.
            v.choose(0, false, cx);
            (v.key_callback())(
                v,
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("ctrl-a").unwrap(),
                    is_held: false,
                },
                window,
                cx,
            );
            assert_eq!(v.draft.selection(), 0..original.len());
            v.choose(0, false, cx);
            for (mac, chord) in [(false, "cmd-a"), (true, "alt-cmd-a")] {
                command(v, chord, mac, window, cx);
                assert_eq!(v.draft.selection(), 0..0);
            }
            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
            for chord in ["cmd-c", "cmd-x"] {
                command(v, chord, true, window, cx);
            }
            assert_eq!(v.draft.text(), original);
            assert_eq!(
                cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                Some("sentinel")
            );
            for transition in ["room", "hidden"] {
                let old = v.key_callback_for_target(true);
                if transition == "room" {
                    v.switch_room(
                        RoomScope {
                            account_id: matrix_domain::AccountId::new(2),
                            room_id: matrix_domain::RoomId::new("!replacement:fixture.invalid"),
                        },
                        cx,
                    );
                    v.replace_text_in_range(None, &original, window, cx);
                } else {
                    v.visit.set_visible(false);
                }
                for chord in ["cmd-a", "cmd-x", "cmd-v", "cmd-z", "enter"] {
                    old(
                        v,
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse(chord).unwrap(),
                            is_held: false,
                        },
                        window,
                        cx,
                    );
                }
                assert_eq!(v.draft.text(), original);
                if transition == "hidden" {
                    command(v, "cmd-v", true, window, cx);
                    v.visit.set_visible(true);
                    old(
                        v,
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse("cmd-x").unwrap(),
                            is_held: false,
                        },
                        window,
                        cx,
                    );
                    assert_eq!(v.draft.text(), original);
                }
            }
            v.replace_and_mark_text_in_range(
                Some(0..original.encode_utf16().count()),
                "preedit 🦀",
                None,
                window,
                cx,
            );
            let marked = v.draft.text().to_owned();
            let selected = v.draft.selection();
            for chord in [
                "cmd-a",
                "cmd-c",
                "cmd-x",
                "cmd-v",
                "cmd-z",
                "cmd-y",
                "cmd-enter",
                "enter",
            ] {
                command(v, chord, true, window, cx);
                assert_eq!(v.draft.text(), marked);
                assert_eq!(v.draft.selection(), selected);
                assert!(v.draft.marked().is_some());
            }
            assert_eq!(
                cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
                Some("sentinel")
            );
            v.unmark_text(window, cx);
            command(v, "ctrl-enter", true, window, cx);
            assert_eq!(v.draft.text(), marked, "Ctrl+Enter policy unchanged");
            let enter = v.key_callback_for_target(true);
            enter(
                v,
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("cmd-enter").unwrap(),
                    is_held: false,
                },
                window,
                cx,
            );
            assert!(
                v.draft.text().is_empty(),
                "existing Cmd+Enter still submits"
            );
            v.replace_text_in_range(None, "replacement", window, cx);
            enter(
                v,
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("cmd-enter").unwrap(),
                    is_held: true,
                },
                window,
                cx,
            );
            assert_eq!(
                v.draft.text(),
                "replacement",
                "retired Enter cannot submit a replacement draft"
            );
            cx.write_to_clipboard(ClipboardItem::new_string("".into()));
        });
    }

    #[test]
    #[cfg(feature = "live")]
    #[ignore = "requires private display"]
    fn mac_editing_never_submits_and_cmd_enter_emits_once() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let observed = Rc::new(std::cell::RefCell::new(Vec::new()));
            let events = observed.clone();
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    cx.new(|cx| {
                        cx.subscribe(&cx.entity(), move |_, _, event: &Submitted, _| {
                            events.borrow_mut().push(event.0.clone())
                        })
                        .detach();
                        let mut v = ComposerView::new(
                            RoomScope {
                                account_id: matrix_domain::AccountId::new(1),
                                room_id: matrix_domain::RoomId::new("!commands:fixture.invalid"),
                            },
                            cx,
                        );
                        v.set_live(true);
                        v.replace_text_in_range(None, "caffè 🦀\nseconda riga", window, cx);
                        v.focus.focus(window);
                        v
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                // Seed a compositor-visible selection so Wayland clipboard reads work.
                executor.timer(std::time::Duration::from_millis(300)).await;
                cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("seed".into())))
                    .unwrap();
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, |v, window, cx| {
                    for chord in [
                        "cmd-a",
                        "cmd-c",
                        "cmd-x",
                        "cmd-v",
                        "cmd-z",
                        "shift-cmd-z",
                        "cmd-z",
                        "cmd-y",
                    ] {
                        command(v, chord, true, window, cx);
                    }
                    assert_eq!(v.draft.text(), "caffè 🦀\nseconda riga");
                    cx.write_to_clipboard(ClipboardItem::new_string("".into()));
                })
                .unwrap();
                executor.timer(std::time::Duration::from_millis(30)).await;
                assert!(
                    observed.borrow().is_empty(),
                    "editing commands never emit Submitted in live mode"
                );
                view.update(cx, |v, window, cx| {
                    let enter = v.key_callback_for_target(true);
                    for _ in 0..3 {
                        enter(
                            v,
                            &KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("cmd-enter").unwrap(),
                                is_held: false,
                            },
                            window,
                            cx,
                        );
                    }
                    v.replace_text_in_range(
                        Some(0..v.draft.text().encode_utf16().count()),
                        "replacement",
                        window,
                        cx,
                    );
                    enter(
                        v,
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse("cmd-enter").unwrap(),
                            is_held: true,
                        },
                        window,
                        cx,
                    );
                    assert_eq!(v.draft.text(), "replacement");
                })
                .unwrap();
                executor.timer(std::time::Duration::from_millis(30)).await;
                assert_eq!(
                    *observed.borrow(),
                    ["caffè 🦀\nseconda riga"],
                    "one exact Submitted, no repeat/replacement send"
                );
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    #[test]
    #[ignore = "requires a private display"]
    fn captured_send_rejects_replacement_room_before_draft_or_focus_change() {
        send_lifecycle(false);
    }

    #[test]
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    #[ignore = "requires a private display; source-bound paint/focus regression"]
    fn painted_send_bounds_and_focus_retire_before_hidden_roundtrip_returns() {
        send_lifecycle(true);
    }

    fn send_lifecycle(check_bounds: bool) {
        let _ = check_bounds;
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let view = cx.open_window(gpui::WindowOptions {
                titlebar: Some(gpui::TitlebarOptions { title: Some("Gnobbi".into()), ..Default::default() }),
                ..Default::default()
            }, |window, cx| cx.new(|cx| {
                let mut v = ComposerView::new(RoomScope {
                    account_id: matrix_domain::AccountId::new(1),
                    room_id: matrix_domain::RoomId::new("!first:fixture.invalid"),
                }, cx);
                v.draft.replace(None, " caffè 🦀\nbozza ");
                v.changed(cx);
                v.focus.focus(window);
                v
            })).unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
            executor.timer(std::time::Duration::from_millis(300)).await;
            view.update(cx, |v, window, cx| {
                #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
                if check_bounds {
                    let scope = RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!first:fixture.invalid") };
                    assert!(v.semantic_snapshot(scope.clone(), window).send_bounds.is_some(), "initial paint publishes Send bounds");
                    v.focus_send(window, cx);
                    v.visit.set_visible(false);
                    v.visit.set_visible(true);
                    let retired = v.semantic_snapshot(scope, window);
                    assert!(retired.send_bounds.is_none(), "unpainted away-back must retire painted bounds");
                    assert!(!retired.send_focused, "unpainted away-back must retire Send focus");
                }
                let old = v.send_callback();
                v.switch_room(RoomScope {
                    account_id: matrix_domain::AccountId::new(1),
                    room_id: matrix_domain::RoomId::new("!second:fixture.invalid"),
                }, cx);
                v.draft.replace(None, " caffè 🦀\nbozza ");
                v.changed(cx);
                let outside = cx.focus_handle();
                outside.focus(window);
                old(v, window, cx);
                assert_eq!(v.draft.text(), " caffè 🦀\nbozza ", "stale Send must not consume identical replacement draft");
                assert!(outside.is_focused(window), "stale Send must not steal focus");
                v.switch_room(RoomScope {
                    account_id: matrix_domain::AccountId::new(1),
                    room_id: matrix_domain::RoomId::new("!first:fixture.invalid"),
                }, cx);
                old(v, window, cx);
                assert_eq!(v.draft.text(), " caffè 🦀\nbozza ");
                assert!(outside.is_focused(window));
                let old_draft = v.send_callback();
                v.draft.replace(Some(0..v.draft.text().len()), " nuova città 🌍 ");
                v.changed(cx);
                v.draft.replace(Some(0..v.draft.text().len()), " caffè 🦀\nbozza ");
                v.changed(cx);
                old_draft(v, window, cx);
                assert_eq!(v.draft.text(), " caffè 🦀\nbozza ");
                assert!(outside.is_focused(window));
                let old_account = v.send_callback();
                let old_account_key = v.key_callback();
                v.switch_room(RoomScope { account_id: matrix_domain::AccountId::new(2), room_id: matrix_domain::RoomId::new("!first:fixture.invalid") }, cx);
                v.draft.replace(None, " caffè 🦀\nbozza "); v.changed(cx);
                old_account(v, window, cx);
                v.switch_room(RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!first:fixture.invalid") }, cx);
                old_account(v, window, cx);
                old_account_key(v, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                assert_eq!(v.draft.text(), " caffè 🦀\nbozza ", "same room/body across account away-back must retire callbacks");
                assert!(outside.is_focused(window));
                let hidden_send = v.send_callback();
                let hidden_key = v.key_callback();
                v.visit.set_visible(false);
                hidden_send(v, window, cx);
                v.visit.set_visible(true);
                hidden_send(v, window, cx);
                hidden_key(v, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                assert_eq!(v.draft.text(), " caffè 🦀\nbozza ", "unpainted hidden roundtrip must retire both production factories");
                assert!(outside.is_focused(window));
                (v.send_callback())(v, window, cx);
                assert!(v.draft.text().is_empty(), "fresh callback still submits");
                eprintln!("captured production Send: identical room/account/draft away-back rejected before consumption/focus; fresh submits");
                if check_bounds {
                    v.replace_text_in_range(None, " nuova città 🌍 ", window, cx);
                }
            }).unwrap();
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            if check_bounds {
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, |v, window, cx| {
                    let scope = RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!first:fixture.invalid") };
                    let current = v.semantic_snapshot(scope.clone(), window);
                    let bounds = current.send_bounds.expect("fresh repaint must reacquire Send bounds");
                    assert!(bounds.width() > 0. && bounds.height() > 0.);
                    assert!(!current.send_focused, "retired focus must not revive on repaint");
                    assert_eq!(v.send_focus_source, v.visit.source().unwrap());
                    v.focus_send(window, cx);
                    assert!(v.semantic_snapshot(scope, window).send_focused);
                    assert_eq!(v.draft.text(), " nuova città 🌍 ");
                    (v.send_callback())(v, window, cx);
                    assert!(v.draft.text().is_empty());
                    eprintln!("current repaint: positive source-bound bounds, no revived focus, fresh Focus and exactly-once Click usable");
                }).unwrap();
            }
            cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }

    #[test]
    #[ignore = "requires a private display"]
    fn captured_send_rejects_preedit_source_after_unmark() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let view = cx.open_window(gpui::WindowOptions::default(), |_, cx| cx.new(|cx| {
                ComposerView::new(RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!ime:fixture.invalid") }, cx)
            })).unwrap();
            cx.spawn(async move |cx| {
                view.update(cx, |v, window, cx| {
                    v.replace_and_mark_text_in_range(None, " caffè 🦀\nbozza ", None, window, cx);
                    let pointer = v.send_callback();
                    let key = v.key_callback();
                    v.unmark_text(window, cx);
                    let outside = cx.focus_handle(); outside.focus(window);
                    pointer(v, window, cx);
                    key(v, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                    assert_eq!(v.draft.text(), " caffè 🦀\nbozza ", "preedit callback cannot send a subsequently committed draft");
                    assert!(outside.is_focused(window));
                    (v.send_callback())(v, window, cx);
                    assert!(v.draft.text().is_empty());
                    eprintln!("IME-unmark: stale pointer/key preserve committed draft and focus; fresh submits");
                }).unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }

    #[test]
    #[cfg(feature = "live")]
    #[ignore = "requires a private display"]
    fn captured_send_emits_once_and_preserves_replacement_draft_while_pending() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let emitted = Rc::new(Cell::new(0));
            let observed = emitted.clone();
            let scope = RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!pending:fixture.invalid") };
            let view = cx.open_window(gpui::WindowOptions::default(), |_, cx| cx.new(|cx| {
                cx.subscribe(&cx.entity(), move |_, _, _: &Submitted, _| observed.set(observed.get() + 1)).detach();
                let mut v = ComposerView::new(scope.clone(), cx);
                v.set_live(true);
                v
            })).unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                view.update(cx, |v, window, cx| {
                    let mut delivery = matrix_desktop::LiveDelivery::new();
                    v.replace_text_in_range(None, " caffè 🦀\nbozza ", window, cx);
                    let pointer = v.send_callback();
                    let key = v.key_callback();
                    pointer(v, window, cx);
                    pointer(v, window, cx);
                    key(v, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                    v.accept_live(&mut delivery, scope.clone(), matrix_domain::TransactionId::new("p098-pending"), cx);
                    let request = delivery.take_request().unwrap();
                    assert_eq!(request.scope, scope);
                    assert_eq!(request.body, " caffè 🦀\nbozza ");
                    assert!(delivery.take_request().is_none());
                    v.replace_text_in_range(None, " nuova città 🌍 ", window, cx);
                    let outside = cx.focus_handle(); outside.focus(window);
                    pointer(v, window, cx);
                    key(v, &KeyDownEvent { keystroke: gpui::Keystroke::parse("enter").unwrap(), is_held: false }, window, cx);
                    assert_eq!(v.draft.text(), " nuova città 🌍 ");
                    assert!(outside.is_focused(window));
                    assert_eq!(delivery.items(&scope).len(), 1);
                }).unwrap();
                executor.timer(std::time::Duration::from_millis(50)).await;
                assert_eq!(emitted.get(), 1, "retired callbacks must not emit a duplicate or replacement submission");
                eprintln!("live composer: exactly one Submitted, exact scoped pending request, replacement draft/focus preserved; no network");
                cx.update(|cx| cx.quit()).unwrap();
            }).detach();
        });
    }

    #[test]
    #[ignore = "requires a private display"]
    fn composer_send_tab_boundary_preserves_draft_and_editor_enter() {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(|cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |window, cx| {
                    cx.new(|cx| {
                        let scope = RoomScope {
                            account_id: matrix_domain::AccountId::new(1),
                            room_id: matrix_domain::RoomId::new("!p089:fixture.invalid"),
                        };
                        let mut v = ComposerView::new(scope, cx);
                        v.draft.replace(None, " caffè 🦀\nbozza ");
                        v.focus.focus(window);
                        v
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                executor.timer(std::time::Duration::from_millis(300)).await;
                view.update(cx, |v, window, cx| {
                    let key = |key: &str| KeyDownEvent {
                        keystroke: gpui::Keystroke::parse(key).unwrap(),
                        is_held: false,
                    };
                    v.key_down(&key("tab"), window, cx);
                    assert!(
                        v.send_focus.is_focused(window),
                        "Tab must reach Send from the editor"
                    );
                    assert!(
                        v.is_focused(window),
                        "Shell must leave keys with the focused composer/send pair"
                    );
                    assert_eq!(v.draft.text(), " caffè 🦀\nbozza ");
                    v.key_down(&key("enter"), window, cx);
                    assert_eq!(
                        v.draft.text(),
                        " caffè 🦀\nbozza ",
                        "send key-down must not race GPUI key-up Click"
                    );
                    v.key_down(&key("shift-tab"), window, cx);
                    assert!(
                        v.focus.is_focused(window),
                        "Shift-Tab must return to editor"
                    );
                    v.key_down(&key("shift-enter"), window, cx);
                    assert!(v.draft.text().ends_with('\n'));
                    v.key_down(&key("enter"), window, cx);
                    assert!(
                        v.draft.text().is_empty(),
                        "editor Enter retains existing submission behavior"
                    );
                    v.key_down(&key("tab"), window, cx);
                    assert!(
                        v.focus.is_focused(window),
                        "empty draft must not enter disabled Send"
                    );
                    v.draft.mark(None, "preedit", None);
                    v.key_down(&key("tab"), window, cx);
                    assert!(v.focus.is_focused(window), "IME owns Tab while composing");
                    eprintln!("actual GPUI composer Tab/ShiftTab/Enter/IME assertions passed");
                })
                .unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    /// One live composer; for each case `attach_paths` runs and, once the file read is done, the
    /// pending file name and whether an error line is shown are compared with the expectation.
    fn attach_cases(cases: Vec<(Vec<std::path::PathBuf>, Option<&'static str>, bool)>) {
        assert_eq!(
            std::env::var("MATRIX_DESKTOP_ISOLATED_RESIZE").as_deref(),
            Ok("1")
        );
        gpui::Application::new().run(move |cx| {
            let view = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        let mut v = ComposerView::new(
                            RoomScope {
                                account_id: matrix_domain::AccountId::new(1),
                                room_id: matrix_domain::RoomId::new("!attach:fixture.invalid"),
                            },
                            cx,
                        );
                        v.set_live(true);
                        v
                    })
                })
                .unwrap();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                for (paths, name, error) in cases {
                    let shown = format!("{paths:?}");
                    view.update(cx, |v, _, cx| {
                        v.attachment = None;
                        v.attach_error = None;
                        v.attach_paths(paths, cx);
                    })
                    .unwrap();
                    executor.timer(std::time::Duration::from_millis(400)).await;
                    view.update(cx, |v, _, _| {
                        assert_eq!(
                            v.attachment.as_ref().map(|p| p.filename.as_str()),
                            name,
                            "{shown}"
                        );
                        assert_eq!(v.attach_error.is_some(), error, "{shown}");
                    })
                    .unwrap();
                }
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gnobbi-attach-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[ignore = "requires private display"]
    fn at_completion_navigates_with_keys_and_inserts_the_pick() {
        command_probe(|v, window, cx| {
            let member = |id: &str, name: &str| matrix_engine::operations::Member {
                user_id: id.into(),
                name: name.into(),
            };
            v.set_live(true);
            v.set_members(
                vec![member("@a:x", "alice"), member("@b:x", "Bruno Verdi")],
                cx,
            );
            v.replace_text_in_range(None, "", window, cx);
            v.draft.select(0..v.draft.text().len());
            v.draft.replace(None, "ciao @");
            v.changed(cx);
            assert_eq!(v.mention_options().len(), 2);
            command(v, "down", false, window, cx);
            assert_eq!(v.mention_selected, 1);
            command(v, "down", false, window, cx);
            assert_eq!(v.mention_selected, 0);
            command(v, "up", false, window, cx);
            assert_eq!(v.mention_selected, 1);
            command(v, "enter", false, window, cx);
            assert_eq!(v.draft.text(), "ciao @Bruno Verdi ");
            assert_eq!(
                v.mentions,
                vec![matrix_engine::sending::Mention {
                    user_id: "@b:x".into(),
                    label: "@Bruno Verdi".into()
                }]
            );
            assert!(v.mention_options().is_empty());
            // Esc closes the list for that @ and leaves the text alone.
            v.draft.replace(None, "@");
            v.changed(cx);
            assert_eq!(v.mention_options().len(), 2);
            command(v, "escape", false, window, cx);
            assert!(v.mention_options().is_empty());
            assert_eq!(v.draft.text(), "ciao @Bruno Verdi @");
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn emoji_picker_inserts_at_the_caret_and_closes_with_esc() {
        command_probe(|v, window, cx| {
            v.set_live(true);
            v.replace_text_in_range(None, "", window, cx);
            v.draft.select(0..v.draft.text().len());
            v.draft.replace(None, "ciao  fine");
            v.draft.select(5..5);
            v.changed(cx);
            v.emoji_open = true;
            v.pick_emoji(0, window, cx);
            assert_eq!(v.draft.text(), "ciao 😀 fine");
            assert!(!v.emoji_open);
            // A selection is replaced, like typing; Esc closes without touching the text.
            v.draft.select(0..4);
            v.emoji_open = true;
            v.pick_emoji(16, window, cx);
            assert_eq!(v.draft.text(), "👍 😀 fine");
            v.emoji_open = true;
            command(v, "escape", false, window, cx);
            assert!(!v.emoji_open);
            assert_eq!(v.draft.text(), "👍 😀 fine");
            // Arrow keys move through the set, Enter inserts the selected one at the caret.
            v.draft.select(4..4);
            v.changed(cx);
            v.emoji_open = true;
            v.emoji_selected = 0;
            command(v, "right", false, window, cx);
            command(v, "enter", false, window, cx);
            assert_eq!(v.draft.text(), "👍😁 😀 fine");
            assert!(!v.emoji_open);
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn dropped_paths_keep_only_a_readable_file_within_the_cap() {
        let dir = scratch("paths");
        let (good, other, big) = (
            dir.join("uno.txt"),
            dir.join("due.txt"),
            dir.join("grande.bin"),
        );
        std::fs::write(&good, b"uno").unwrap();
        std::fs::write(&other, b"due").unwrap();
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_ATTACHMENT + 1)
            .unwrap(); // sparse: only the size is read
        attach_cases(vec![
            // Several files dropped together: only the first becomes the pending file, the rest is ignored.
            (vec![good.clone(), other], Some("uno.txt"), false),
            (vec![dir.clone()], None, true),           // a folder
            (vec![dir.join("manca.txt")], None, true), // vanished before the read
            (vec![big], None, true),                   // one byte over the cap
            (vec![], None, false),                     // nothing dropped
        ]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "requires private display"]
    fn empty_file_is_refused_before_sending() {
        let dir = scratch("empty");
        let empty = dir.join("vuoto.txt");
        std::fs::write(&empty, b"").unwrap();
        // A zero-byte upload is rejected by Synapse, so the file must not become attachable.
        attach_cases(vec![(vec![empty], None, true)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "requires private display"]
    fn pasting_an_image_attaches_it_and_text_does_not() {
        command_probe(|v, window, cx| {
            let mut png = Vec::new();
            image::DynamicImage::new_rgb8(4, 3)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            let picture = gpui::Image::from_bytes(gpui::ImageFormat::Png, png);
            let before = v.draft.text().to_owned();
            // Not connected: the image is not taken.
            cx.write_to_clipboard(ClipboardItem::new_image(&picture));
            command(v, "ctrl-v", false, window, cx);
            assert!(v.attachment.is_none());
            v.set_live(true);
            command(v, "ctrl-v", false, window, cx);
            let pending = v
                .attachment
                .as_ref()
                .expect("pasted image becomes the pending file");
            assert_eq!(pending.filename, "immagine.png");
            assert_eq!(
                v.draft.text(),
                before,
                "the image is not also pasted as text"
            );
            // Text afterwards goes to the draft and leaves the pending image alone.
            cx.write_to_clipboard(ClipboardItem::new_string("incollato".into()));
            command(v, "ctrl-v", false, window, cx);
            assert!(v.attachment.is_some());
            assert!(v.draft.text().contains("incollato"));
        });
    }

    #[test]
    #[ignore = "requires private display"]
    fn recording_without_microphone_or_interrupted_leaves_no_half_note() {
        command_probe(|v, window, cx| {
            v.set_live(true);
            let scope = RoomScope {
                account_id: matrix_domain::AccountId::new(1),
                room_id: matrix_domain::RoomId::new("!other:fixture.invalid"),
            };
            for _ in 0..3 {
                v.start_recording(cx);
                if v.recorder.is_none() {
                    // No input device or no permission: one error line, nothing pending.
                    assert_eq!(v.attach_error, Some("Microfono non disponibile"));
                    assert!(v.attachment.is_none());
                    continue;
                }
                v.stop_recording(false, cx);
                assert!(
                    v.recorder.is_none() && v.attachment.is_none(),
                    "cancel discards the note"
                );
                v.start_recording(cx);
                v.switch_room(scope.clone(), cx);
                assert!(
                    v.recorder.is_none() && v.attachment.is_none(),
                    "leaving the room drops the recording"
                );
            }
            // Stopping with nothing recording is a no-op.
            v.stop_recording(true, cx);
            v.stop_recording(false, cx);
            assert!(v.recorder.is_none());
            let _ = window;
        });
    }
}
