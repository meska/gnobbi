//! «Recupera chiavi» on an undecryptable bubble: one control, and under it the key field once opened.
use crate::login_field::LoginField;
use gpui::{AnyElement, App, Entity, FocusHandle, Window, canvas, div, prelude::*, px, rgb};
use matrix_desktop::{
    DesignTokens,
    recover_keys::{RECOVER_KEYS_LABEL, RECOVERY_KEY_LABEL, RecoverKeys},
};
use matrix_domain::ScopedTimelineId;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub(crate) type Ui = Rc<RefCell<RecoverKeys>>;
/// The single key field, created on first open.
pub(crate) type Field = Rc<RefCell<Option<(Entity<LoginField>, FocusHandle)>>>;
/// One tab stop per bubble, kept across frames so the focus survives a repaint.
pub(crate) type Focus = Rc<RefCell<HashMap<ScopedTimelineId, FocusHandle>>>;
/// Reports the painted bounds of the control (`false`) or the field (`true`).
pub(crate) type Painted = Rc<dyn Fn(bool, gpui::Bounds<gpui::Pixels>)>;

/// Opens the field under `id` and puts the key focus in it.
pub(crate) fn open(
    ui: &Ui,
    field: &Field,
    id: &ScopedTimelineId,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if !ui.borrow_mut().open(id) {
        return false;
    }
    let (entity, focus) = field
        .borrow_mut()
        .get_or_insert_with(|| {
            let entity = cx.new(|cx| {
                let mut field = LoginField::new(true, cx);
                field.enabled = true;
                field.placeholder = RECOVERY_KEY_LABEL;
                field
            });
            let focus = entity.read(cx).focus.clone();
            (entity, focus)
        })
        .clone();
    entity.update(cx, |f, _| drop(f.input.take()));
    focus.focus(window);
    window.refresh();
    true
}

pub(crate) fn row(
    index: usize,
    id: &ScopedTimelineId,
    t: DesignTokens,
    (ui, field, focus): (&Ui, &Field, &Focus),
    painted: Painted,
    cx: &mut App,
) -> Option<AnyElement> {
    let (is_open, status, pending) = {
        let ui = ui.borrow();
        if !ui.offered() {
            return None;
        }
        (ui.is_open(id), ui.status(), ui.pending())
    };
    let handle = focus
        .borrow_mut()
        .entry(id.clone())
        .or_insert_with(|| cx.focus_handle().tab_stop(true))
        .clone();
    let control = {
        let (ui, field, id, painted) = (ui.clone(), field.clone(), id.clone(), painted.clone());
        div()
            .id(("recover-keys", index))
            .relative()
            .px_1()
            .text_size(px(12.))
            .text_color(rgb(t.accent))
            .cursor_pointer()
            .track_focus(&handle)
            .focus(|s| s.bg(rgb(t.border)))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                open(&ui, &field, &id, window, cx);
            })
            .child(RECOVER_KEYS_LABEL)
            .child(
                canvas(|_, _, _| (), move |bounds, _, _, _| painted(false, bounds))
                    .absolute()
                    .inset_0(),
            )
    };
    let entity = field.borrow().clone().filter(|_| is_open);
    let field_row = entity.map(|(entity, _)| {
        entity.update(cx, |f, _| f.theme = t);
        let (ui, handle, painted, source) =
            (ui.clone(), handle.clone(), painted.clone(), entity.clone());
        div()
            .id(("recover-field", index))
            .relative()
            .w(px(260.))
            .on_key_down(
                move |event, window, cx| match event.keystroke.key.as_str() {
                    "enter" => {
                        cx.stop_propagation();
                        // The key is taken only when an attempt may start; the field is empty right after.
                        if ui.borrow().accepts_key() {
                            let key = source.update(cx, |f, _| f.input.take());
                            ui.borrow_mut().submit(key);
                            window.refresh();
                        }
                    }
                    "escape" => {
                        cx.stop_propagation();
                        source.update(cx, |f, _| drop(f.input.take()));
                        ui.borrow_mut().close();
                        handle.focus(window);
                        window.refresh();
                    }
                    _ => {}
                },
            )
            .child(entity)
            .child(
                canvas(|_, _, _| (), move |bounds, _, _, _| painted(true, bounds))
                    .absolute()
                    .inset_0(),
            )
    });
    Some(
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .child(control)
            .children(field_row)
            .children(status.filter(|_| is_open).map(|s| {
                div()
                    .text_size(px(12.))
                    .text_color(rgb(if pending { t.secondary } else { t.error }))
                    .child(s)
            }))
            .into_any_element(),
    )
}

impl crate::Shell {
    /// «Recupera chiavi» for this frame: offered, Pending, last failure line.
    pub(crate) fn key_recovery_state(&self) -> (bool, bool, Option<&'static str>) {
        #[cfg(feature = "live")]
        {
            let pending = self.verification.key_recovery()
                == Some(matrix_engine::signin::RecoveryState::Pending);
            (
                self.verification.offers_recover_keys(true),
                pending,
                self.verification.key_recovery_message(),
            )
        }
        #[cfg(not(feature = "live"))]
        {
            (false, false, None)
        }
    }

    /// Queues the key typed under a bubble; the flush of the verification route (next in the pump) sends it.
    pub(crate) fn drive_recover_keys(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(feature = "live")]
        if let Some(key) = self
            .timeline
            .as_ref()
            .and_then(|timeline| timeline.take_recovery_key())
        {
            self.verification.recover_keys(key);
            cx.notify();
        }
        #[cfg(not(feature = "live"))]
        let _ = cx;
    }
}
