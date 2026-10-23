//! Native single-line input. Paint and surrounding text use the same projection.
use crate::editing_command::command_modifier;
use gpui::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, KeyDownEvent,
    MouseButton, Pixels, Point, StyledText, TextLayout, UTF16Selection, Window, canvas, div, fill,
    point, prelude::*, px, rgb, size,
};
use matrix_desktop::{LIGHT, LoginInput};
use std::ops::Range;

#[cfg(test)]
type PaintedPointer = fn(&mut LoginField, &mut Window, &mut Context<LoginField>);
#[cfg(test)]
type PaintedKey = fn(&mut LoginField, &KeyDownEvent, &mut Window, &mut Context<LoginField>);

pub struct LoginField {
    pub enabled: bool,
    pub input: LoginInput,
    pub focus: FocusHandle,
    layout: Option<TextLayout>,
    #[cfg(test)]
    pub painted_pointer: Option<PaintedPointer>,
    #[cfg(test)]
    pub painted_key: Option<PaintedKey>,
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub semantic_bounds: Option<accesskit::Rect>,
}

impl LoginField {
    pub fn new(protected: bool, cx: &mut Context<Self>) -> Self {
        Self {
            enabled: false,
            input: if protected {
                LoginInput::secret()
            } else {
                LoginInput::public()
            },
            focus: cx.focus_handle(),
            layout: None,
            #[cfg(test)]
            painted_pointer: None,
            #[cfg(test)]
            painted_key: None,
            #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
            semantic_bounds: None,
        }
    }
    fn pointer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        self.focus.focus(window);
        self.input.edit("end", false, false);
        cx.notify();
        cx.stop_propagation();
    }
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.key_for_platform(event, cfg!(target_os = "macos"), cx);
    }
    fn key_for_platform(&mut self, event: &KeyDownEvent, mac: bool, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        let key = &event.keystroke;
        let command = command_modifier(&key.modifiers, mac);
        let mac_command = mac && key.modifiers.platform && !key.modifiers.alt;
        if mac_command && matches!(key.key.as_str(), "c" | "x") {
            if let Some(text) = self.input.exportable_selection() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                if key.key == "x" {
                    self.input.replace(None, "");
                }
            }
        } else if command && key.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.input.replace(None, &text);
            }
        } else if !self.input.edit(&key.key, command, key.modifiers.shift) {
            return;
        }
        cx.notify();
        cx.stop_propagation();
    }
}
fn byte_offset(text: &str, target: usize) -> Option<usize> {
    let mut offset = 0;
    for (index, c) in text.char_indices() {
        if offset == target {
            return Some(index);
        }
        offset += c.len_utf16();
    }
    (offset == target).then_some(text.len())
}
impl EntityInputHandler for LoginField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.input.text_for_range(range.clone())?;
        *actual = Some(range);
        Some(text)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.input.selection_utf16(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.input.marked_utf16()
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.input.unmark();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        self.input.replace(range, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.enabled {
            self.input.mark(range, text, selected);
            cx.notify();
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let text = self.input.display_text();
        let p = self
            .layout
            .as_ref()?
            .position_for_index(byte_offset(&text, range.start)?)?;
        Some(Bounds::new(p, size(px(1.), px(20.))))
    }
    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let text = self.input.display_text();
        let i = self
            .layout
            .as_ref()?
            .index_for_position(p)
            .unwrap_or_else(|i| i)
            .min(text.len());
        text.get(..i).map(|text| text.encode_utf16().count())
    }
}
impl Render for LoginField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pointer = Self::pointer;
        let key = Self::key;
        #[cfg(test)]
        {
            self.painted_pointer = Some(pointer);
            self.painted_key = Some(key);
        }
        let text = self.input.display_text();
        let cursor = byte_offset(&text, self.input.selection_utf16().end).unwrap_or(text.len());
        let body = StyledText::new(format!("{text}\u{200b}"));
        let layout = body.layout().clone();
        let entity = cx.entity();
        div()
            .id("login-field")
            .track_focus(&self.focus)
            .h(px(36.))
            .flex_shrink_0()
            .relative()
            .overflow_hidden()
            .px_2()
            .py_1()
            .text_size(px(14.))
            .line_height(px(24.))
            .bg(rgb(LIGHT.input))
            .border_1()
            .border_color(rgb(if self.focus.is_focused(window) {
                LIGHT.accent
            } else {
                LIGHT.border
            }))
            .on_key_down(cx.listener(key))
            .when(cfg!(any(target_os = "macos", test)), |element| {
                crate::editing_command::with_actions(
                    element,
                    true,
                    cx.listener(|this, event, window, cx| {
                        if this.enabled && this.focus.is_focused(window) {
                            this.key_for_platform(event, true, cx);
                        }
                    }),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| pointer(this, window, cx)),
            )
            .child(body)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let this = entity.read(cx);
                        window.handle_input(
                            &this.focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        if this.focus.is_focused(window)
                            && let Some(p) = layout.position_for_index(cursor)
                        {
                            window.paint_quad(fill(
                                Bounds::new(point(p.x, p.y), size(px(1.), px(20.))),
                                rgb(LIGHT.accent),
                            ));
                        }
                        entity.update(cx, |this, _| this.layout = Some(layout.clone()));
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
                                cx.defer(move |cx| {
                                    entity.update(cx, |this, cx| {
                                        this.semantic_bounds = Some(rect);
                                        cx.notify();
                                    })
                                });
                            }
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[cfg(all(feature = "perf-probe", feature = "live", target_os = "macos"))]
pub(crate) mod mac_clipboard_probe;

#[cfg(test)]
mod clipboard_test;
