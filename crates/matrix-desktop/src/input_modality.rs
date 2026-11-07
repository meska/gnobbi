//! Last interaction kind: the focus ring is drawn only after keyboard or assistive-technology input.
//! Paint only: the accessibility tree must never read this.
use gpui::{App, FocusHandle, Global, Window};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Modality {
    #[default]
    Pointer,
    Keyboard,
}

struct InputModality(Modality);
impl Global for InputModality {}

/// Pointer until the first key: no ring at startup, nor when the global is missing.
pub(crate) fn is_keyboard(cx: &App) -> bool {
    cx.try_global::<InputModality>().is_some_and(|m| m.0 == Modality::Keyboard)
}

pub(crate) fn set_keyboard(cx: &mut App) {
    set(Modality::Keyboard, cx)
}

pub(crate) fn set_pointer(cx: &mut App) {
    set(Modality::Pointer, cx)
}

fn set(m: Modality, cx: &mut App) {
    if is_keyboard(cx) != (m == Modality::Keyboard) {
        cx.set_global(InputModality(m));
        cx.refresh_windows(); // uncached child views read the new value
    }
}

/// Ring for a control with its own focus handle.
pub(crate) fn ring(handle: &FocusHandle, window: &Window, cx: &App) -> bool {
    is_keyboard(cx) && handle.is_focused(window)
}

/// Gate an already computed "has focus" for painting the ring.
pub(crate) fn only_keys(focused: bool, cx: &App) -> bool {
    focused && is_keyboard(cx)
}
